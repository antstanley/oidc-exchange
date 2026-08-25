//! Service-level leak corpus (plan task 07): the store, refresh, and revoke paths,
//! driven end to end through `AppService` under the shared capturing subscriber with
//! span close events enabled.
//!
//! Two sinks are asserted:
//!
//! 1. **Tracing output** (events and span open/close lines): across a full exchange →
//!    refresh → revoke lifecycle, neither the minted refresh tokens, nor their SHA-256
//!    digests, the presented authorization code, nor any configured secret
//!    (`user_sync.webhook.secret`, `internal_api.shared_secret`) may render — matched
//!    literally *and* after percent-decoding. The mock session store carries no
//!    `#[instrument]` today, so this is also the tripwire for unskipped instrumentation
//!    added to shared test mocks later.
//!
//! 2. **The audit fallback stream**: with the audit adapter forced to fail and the
//!    blocking threshold set so failures never abort an operation, every audit event
//!    serializes into a tracing fallback log line. Those payloads legitimately carry
//!    identifiers (user id, provider, provenance) — and must never carry credentials.

use std::collections::HashMap;

use sha2::{Digest, Sha256};

use oidc_exchange_core::config::{Config, RawConfig};
use oidc_exchange_core::ports::IdentityProvider;
use oidc_exchange_core::service::exchange::{ExchangeCredential, ExchangeRequest};
use oidc_exchange_core::service::refresh::RefreshRequest;
use oidc_exchange_core::service::revoke::RevokeRequest;
use oidc_exchange_core::service::AppService;
use oidc_exchange_test_utils::telemetry::{
    assert_absent_plain_and_encoded, install_span_capture, SharedBuffer,
};
use oidc_exchange_test_utils::{
    MockAuditLog, MockIdentityProvider, MockKeyManager, MockRepository, MockUserSync,
};

// ---------------------------------------------------------------------------
// Sentinels — distinct per value class, obviously fake
// ---------------------------------------------------------------------------

/// Presented authorization code at the exchange boundary.
const CODE_SENTINEL: &str = "SENTINEL-AUTH-CODE-VALUE";
/// Configured webhook HMAC secret (`[user_sync.webhook].secret`).
const WEBHOOK_SECRET_SENTINEL: &str = "sentinel-webhook-hmac-secret-value";
/// Configured internal API bearer secret (`internal_api.shared_secret`).
const SHARED_SECRET_SENTINEL: &str = "sentinel-internal-shared-secret-value";
/// Client provenance planted on requests and stored on sessions; permitted in audit
/// records but never as telemetry values.
const PROVENANCE_SENTINELS: [&str; 3] = [
    "corpus-device-sentinel",
    "corpus-user-agent/1.0",
    "192.0.2.77",
];

fn corpus_config() -> Config {
    let mut raw: RawConfig = toml::from_str(include_str!("../../../config/default.toml"))
        .expect("default config deserializes");
    raw.server.issuer = "https://auth.example.com".to_string();
    // The two configured secrets the service holds in memory for its whole lifetime.
    raw.user_sync.webhook = Some(oidc_exchange_core::config::RawWebhookConfig {
        url: "https://hooks.example.com/notify".to_string(),
        secret: WEBHOOK_SECRET_SENTINEL.to_string(),
        timeout: None,
        retries: None,
    });
    raw.internal_api.shared_secret = Some(SHARED_SECRET_SENTINEL.to_string());
    Config::resolve(raw).expect("corpus config resolves")
}

/// Build a service whose audit adapter is the caller's handle (so fail mode can be
/// toggled per test), over one shared mock repository backing both ports.
fn make_service(config: Config, audit: MockAuditLog) -> AppService {
    let provider = MockIdentityProvider::new("test");
    let provider_id = provider.provider_id().to_string();
    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    providers.insert(provider_id, Box::new(provider));

    let repo = MockRepository::new();
    AppService::new(
        Box::new(repo.clone()),
        Box::new(repo),
        Box::new(MockKeyManager::new()),
        Box::new(audit),
        Box::new(MockUserSync::new()),
        Box::new(oidc_exchange_test_utils::MockRateLimiter::new()),
        providers,
        config,
    )
}

fn provenance_request_fields() -> [Option<String>; 3] {
    [
        Some(PROVENANCE_SENTINELS[0].to_string()),
        Some(PROVENANCE_SENTINELS[1].to_string()),
        Some(PROVENANCE_SENTINELS[2].to_string()),
    ]
}

#[tokio::test]
async fn full_lifecycle_leaks_no_credentials_into_telemetry() {
    let capture = install_span_capture(SharedBuffer::default());
    let service = make_service(corpus_config(), MockAuditLog::new());

    // Positive control: the capture is live before any absence claim means anything.
    tracing::info!(target: "oidc_exchange_corpus", "corpus-marker: lifecycle start");

    // --- store path (exchange mints + stores the session) ---
    let [device, user_agent, ip] = provenance_request_fields();
    let exchange = service
        .exchange(ExchangeRequest {
            credential: ExchangeCredential::AuthorizationCode {
                code: CODE_SENTINEL.to_string(),
                redirect_uri: "https://client.example.com/callback".to_string(),
            },
            provider: "test".to_string(),
            provider_access_token: None,
            ip_address: ip,
            user_agent,
            device_id: device,
        })
        .await
        .expect("exchange_code");
    let refresh_token_one = exchange
        .refresh_token
        .as_ref()
        .expect("exchange must mint a refresh token")
        .expose()
        .clone();
    // Same digest derivation as the service: SHA-256, hex-encoded.
    let hash_one = hex::encode(Sha256::digest(refresh_token_one.as_bytes()));

    // --- refresh path: re-presents the same token for a new access token ---
    let [device, user_agent, ip] = provenance_request_fields();
    let refreshed = service
        .refresh(RefreshRequest {
            refresh_token: refresh_token_one.clone(),
            ip_address: ip,
            user_agent,
            device_id: device,
        })
        .await
        .expect("refresh");
    // Rotation retires the presented generation and mints a replacement; the
    // replacement is one more credential-derived value the telemetry stream
    // must never carry.
    let refresh_token_two = refreshed
        .refresh_token
        .as_ref()
        .expect("rotation must mint a replacement refresh token")
        .expose()
        .clone();
    let hash_two = hex::encode(Sha256::digest(refresh_token_two.as_bytes()));

    // --- revoke paths ---
    let [device, user_agent, ip] = provenance_request_fields();
    service
        .revoke(RevokeRequest {
            token: refresh_token_two.clone(),
            token_type_hint: Some("refresh_token".to_string()),
            ip_address: ip,
            user_agent,
            device_id: device,
        })
        .await
        .expect("revoke of the live token");
    // An unknown token exercises the ValidationFailed-style silent path without
    // surfacing the presented material anywhere.
    service
        .revoke(RevokeRequest {
            token: format!("SENTINEL-UNKNOWN-TOKEN-PREFIX-{refresh_token_one}"),
            token_type_hint: Some("refresh_token".to_string()),
            ip_address: None,
            user_agent: None,
            device_id: None,
        })
        .await
        .expect("revoke of an unknown token stays silent per RFC 7009");

    let rendered = capture.rendered();

    // Non-vacuousness: the marker event rendered under this subscriber.
    assert!(
        rendered.contains("corpus-marker"),
        "the capture must be live for the absence claims below to mean anything"
    );

    // Negative space — every credential-derived value, plain and percent-decoded:
    assert_absent_plain_and_encoded(&rendered, CODE_SENTINEL);
    assert_absent_plain_and_encoded(&rendered, &refresh_token_one);
    assert_absent_plain_and_encoded(&rendered, &hash_one);
    assert_absent_plain_and_encoded(&rendered, &refresh_token_two);
    assert_absent_plain_and_encoded(&rendered, &hash_two);
    assert_absent_plain_and_encoded(&rendered, WEBHOOK_SECRET_SENTINEL);
    assert_absent_plain_and_encoded(&rendered, SHARED_SECRET_SENTINEL);
}

/// With the audit adapter failing but blocking disabled, every audit dispatch falls
/// back to a serialized tracing log line. Those lines are operator-visible telemetry:
/// they may carry identifiers, never credentials. This also proves the corpus reaches
/// a real sink beyond the single marker event.
#[tokio::test]
async fn audit_fallback_payloads_carry_no_credentials() {
    let capture = install_span_capture(SharedBuffer::default());

    let mut config = corpus_config();
    // Syslog-style severities: nothing severe enough to block, everything dispatched
    // (including Debug validation-failure events, which the default emit floor drops).
    config.audit.blocking_threshold = oidc_exchange_core::domain::AuditSeverity::Emergency;
    config.audit.emit_threshold = oidc_exchange_core::domain::AuditSeverity::Debug;

    let audit = MockAuditLog::new();
    audit.set_fail_mode(true).await;
    let service = make_service(config, audit);

    let [device, user_agent, ip] = provenance_request_fields();
    let exchange = service
        .exchange(ExchangeRequest {
            credential: ExchangeCredential::AuthorizationCode {
                code: CODE_SENTINEL.to_string(),
                redirect_uri: "https://client.example.com/callback".to_string(),
            },
            provider: "test".to_string(),
            provider_access_token: None,
            ip_address: ip,
            user_agent,
            device_id: device,
        })
        .await
        .expect("audit failure must not block with the emergency threshold");
    let refresh_token = exchange
        .refresh_token
        .as_ref()
        .expect("minted refresh token")
        .expose()
        .clone();

    // Unknown-token refresh: emits a Debug-severity ValidationFailed carrying only a
    // fixed reason — never the presented token or its digest.
    service
        .refresh(RefreshRequest {
            refresh_token: "SENTINEL-PRESENTED-BUT-UNKNOWN-TOKEN".to_string(),
            ip_address: None,
            user_agent: None,
            device_id: None,
        })
        .await
        .expect_err("an unknown refresh token must be rejected");

    let rendered = capture.rendered();

    // Non-vacuousness: the fallback sink actually fired, with structured payloads.
    assert!(
        rendered.contains("audit_fallback=true"),
        "the audit fallback stream must have been exercised, got {rendered:?}"
    );
    assert!(
        rendered.contains("ValidationFailed") || rendered.contains("validation_failed"),
        "the unknown-token validation failure must have reached the fallback stream"
    );

    assert_absent_plain_and_encoded(&rendered, CODE_SENTINEL);
    assert_absent_plain_and_encoded(&rendered, &refresh_token);
    assert_absent_plain_and_encoded(&rendered, "SENTINEL-PRESENTED-BUT-UNKNOWN-TOKEN");
    assert_absent_plain_and_encoded(&rendered, WEBHOOK_SECRET_SENTINEL);
    assert_absent_plain_and_encoded(&rendered, SHARED_SECRET_SENTINEL);
}

/// The service holds its configured secrets for its whole lifetime; a panic backtrace,
/// a stray debug print, or a future log statement that formats the config must surface
/// here first. Drives a trivial flow and asserts the secrets render nowhere.
#[tokio::test]
async fn configured_secrets_never_render_during_normal_operation() {
    let capture = install_span_capture(SharedBuffer::default());
    let service = make_service(corpus_config(), MockAuditLog::new());

    service
        .exchange(ExchangeRequest {
            credential: ExchangeCredential::AuthorizationCode {
                code: "irrelevant-code".to_string(),
                redirect_uri: "https://client.example.com/callback".to_string(),
            },
            provider: "test".to_string(),
            provider_access_token: None,
            ip_address: None,
            user_agent: None,
            device_id: None,
        })
        .await
        .expect("exchange succeeds");

    let rendered = capture.rendered();
    assert_absent_plain_and_encoded(&rendered, WEBHOOK_SECRET_SENTINEL);
    assert_absent_plain_and_encoded(&rendered, SHARED_SECRET_SENTINEL);
}
