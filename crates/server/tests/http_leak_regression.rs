//! End-to-end HTTP token-lifecycle leak corpus (plan task 07).
//!
//! The other task-07 suites cover the request-ID boundaries, the public error oracle,
//! the service-level flows, and the provider upstream paths; this file closes the one
//! gap they leave: the **store → refresh → revoke lifecycle driven over real HTTP
//! against a real instrumented session store** (LMDB), so the adapter spans those
//! methods open are rendered by a capturing subscriber with `FmtSpan::NEW | CLOSE`
//! while the router carries the production middleware order (request id outermost,
//! audit context inside it).
//!
//! Provenance sentinels ride in through the audit-context headers on every request;
//! the raw refresh token is taken from the `/token` response and its SHA-256 hex
//! digest (the stored session lookup key) is derived the same way the core derives it.
//! Nothing in that set may render anywhere in telemetry — literal or percent-decoded —
//! while the permitted fields (`user_id`, the declared-but-valueless `token_hash`
//! schema entries) stay observable.
//!
//! Single-threaded `#[tokio::test]` runtimes keep every poll on the installing thread,
//! so the thread-local capturing subscriber sees every span open and close.

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::middleware::from_fn;
use axum::Router;
use http_body_util::BodyExt;
use sha2::{Digest, Sha256};
use tower::ServiceExt;

use oidc_exchange::middleware::audit_context::ffi_audit_context_layer;
use oidc_exchange::middleware::request_id::request_id_layer;
use oidc_exchange::routes::public_routes;
use oidc_exchange::state::AppState;
use oidc_exchange_adapters::lmdb::LmdbSessionRepository;
use oidc_exchange_core::config::{Config, RawConfig};
use oidc_exchange_core::ports::IdentityProvider;
use oidc_exchange_core::service::AppService;
use oidc_exchange_test_utils::telemetry::{
    assert_absent_plain_and_encoded, install_span_capture, SharedBuffer,
};
use oidc_exchange_test_utils::{
    MockAuditLog, MockIdentityProvider, MockKeyManager, MockRepository, MockUserSync,
};

/// Distinct provenance sentinels planted via audit-context headers on every request.
const PROVENANCE_IP: &str = "192.0.2.77";
const PROVENANCE_UA: &str = "leak-corpus-agent/7.3";
const PROVENANCE_DEVICE: &str = "leak-corpus-device-99";

/// Router plus the LMDB environment handle that must stay alive for the whole test.
struct CorpusApp {
    app: Router,
    _dir: tempfile::TempDir,
}

async fn build_corpus_app() -> CorpusApp {
    let dir = tempfile::TempDir::new().expect("temp dir for lmdb environment");
    let db_path = dir.path().join("sessions.lmdb");
    let lmdb = LmdbSessionRepository::new(db_path.to_str().expect("utf-8 temp path"), 16, 3600)
        .expect("open lmdb session environment");

    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    providers.insert(
        "corpus-provider".to_string(),
        Box::new(MockIdentityProvider::new("corpus-provider")),
    );

    let mut raw: RawConfig = toml::from_str(include_str!("../../../config/default.toml"))
        .expect("default config deserializes");
    raw.server.issuer = "https://auth.example.com".to_string();
    let config = Config::resolve(raw).expect("test config resolves");

    let service = AppService::new(
        Box::new(MockRepository::new()),
        Box::new(lmdb),
        Box::new(MockKeyManager::new()),
        Box::new(MockAuditLog::new()),
        Box::new(MockUserSync::new()),
        Box::new(oidc_exchange_test_utils::MockRateLimiter::new()),
        providers,
        config.clone(),
    );

    let state = AppState {
        service: Arc::new(service),
        config: Arc::new(config),
        rate_limiter: std::sync::Arc::new(oidc_exchange_adapters::noop::NoopRateLimiter::new()),
    };

    // Layer order mirrors production: request id outermost so every downstream span —
    // including the LMDB store spans asserted below — sits inside the request span.
    let app = public_routes()
        .layer(from_fn(ffi_audit_context_layer))
        .layer(from_fn(request_id_layer))
        .with_state(state);

    CorpusApp { app, _dir: dir }
}

/// POST an urlencoded form to `path`, planting the provenance sentinels the way the
/// audit-context middleware expects to find them.
fn form_request(path: &'static str, form: String) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/x-www-form-urlencoded")
        .header("x-forwarded-for", PROVENANCE_IP)
        .header("user-agent", PROVENANCE_UA)
        .header("x-device-id", PROVENANCE_DEVICE)
        .body(Body::from(form))
        .expect("form request builds")
}

async fn body_to_json(body: Body) -> serde_json::Value {
    let bytes = body
        .collect()
        .await
        .expect("response body collects")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("response body is JSON")
}

/// Raw response body as text, for endpoints whose success bodies are deliberately
/// empty (`/revoke` per RFC 7009).
async fn response_body_string(body: Body) -> String {
    let bytes = body
        .collect()
        .await
        .expect("response body collects")
        .to_bytes();
    String::from_utf8(bytes.to_vec()).expect("response body is utf-8")
}

/// SHA-256 hex digest, exactly as core derives the session lookup key.
fn hex_encode_sha256(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

/// Drive exchange → refresh → revoke over HTTP against the real LMDB session store and
/// prove that neither the raw refresh token, nor its stored digest, nor any provenance
/// sentinel reaches telemetry — while the permitted observability stays intact.
#[tokio::test]
async fn token_lifecycle_leaks_nothing_into_telemetry() {
    let corpus = build_corpus_app().await;
    let capture = install_span_capture(SharedBuffer::default());

    // --- exchange: mints the refresh token and stores its hash in LMDB ---
    let exchange_response = corpus
        .app
        .clone()
        .oneshot(form_request(
            "/token",
            "grant_type=authorization_code&code=corpus-code&redirect_uri=http://localhost/callback&provider=corpus-provider"
                .to_string(),
        ))
        .await
        .expect("in-flight request");
    assert_eq!(exchange_response.status(), StatusCode::OK);
    let tokens = body_to_json(exchange_response.into_body()).await;
    let raw_refresh_token = tokens["refresh_token"]
        .as_str()
        .expect("exchange returns a refresh token")
        .to_string();
    // Shape control: the minted token is 32 random bytes, base64url-encoded, so the
    // absence claims below are about a value that really flowed through the flow.
    assert_eq!(
        raw_refresh_token.len(),
        43,
        "a 256-bit base64url refresh token is 43 characters"
    );
    let token_hash_hex = hex_encode_sha256(&raw_refresh_token);

    // --- refresh: presents the same token; the LMDB lookup span fires ---
    let refresh_response = corpus
        .app
        .clone()
        .oneshot(form_request(
            "/token",
            format!("grant_type=refresh_token&refresh_token={raw_refresh_token}"),
        ))
        .await
        .expect("in-flight request");
    assert_eq!(refresh_response.status(), StatusCode::OK);
    let refreshed = body_to_json(refresh_response.into_body()).await;
    assert_eq!(refreshed["token_type"], "Bearer");
    assert!(
        refreshed["access_token"].is_string(),
        "a refresh must return a fresh access token"
    );
    // Rotation retires the presented generation; the replacement is the live
    // credential the revoke below must present.
    let rotated_refresh_token = refreshed["refresh_token"]
        .as_str()
        .expect("rotation returns a replacement refresh token")
        .to_string();

    // --- revoke: an attacker-chosen unknown token must stay silent (RFC 7009) ---
    let hostile_token = "HOSTILE-REVOCATION-TOKEN-SENTINEL";
    let revoke_unknown = corpus
        .app
        .clone()
        .oneshot(form_request(
            "/revoke",
            format!("token={hostile_token}&token_type_hint=refresh_token"),
        ))
        .await
        .expect("in-flight request");
    assert_eq!(revoke_unknown.status(), StatusCode::OK);
    // RFC 7009: an unknown token gets a plain, empty-bodied 200 — no error envelope,
    // and nothing about the presented token.
    let unknown_bytes = response_body_string(revoke_unknown.into_body()).await;
    assert!(
        unknown_bytes.trim().is_empty(),
        "an unknown token must get an empty-bodied 200 per RFC 7009, got {unknown_bytes:?}"
    );

    // --- revoke: the real refresh token; the LMDB revoke span fires ---
    let revoke_response = corpus
        .app
        .oneshot(form_request(
            "/revoke",
            format!("token={rotated_refresh_token}&token_type_hint=refresh_token"),
        ))
        .await
        .expect("in-flight request");
    assert_eq!(revoke_response.status(), StatusCode::OK);

    let rendered = capture.rendered();

    // Non-vacuousness: the instrumented store spans opened AND closed inside this
    // capture, and the permitted user_id projection rendered.
    for span_name in [
        "store_refresh_token",
        "get_session_by_refresh_token",
        "revoke_session",
    ] {
        let mentions = rendered.matches(span_name).count();
        assert!(
            mentions >= 2,
            "span {span_name} must appear at open and close, found {mentions}"
        );
    }
    let closes = rendered.matches("close").count();
    assert!(
        closes >= 3,
        "span-close events must be enabled for these claims to mean anything, got {closes}"
    );
    assert!(
        rendered.matches("user_id=").count() >= 2,
        "the permitted user_id projection must still render on the write spans"
    );

    use oidc_exchange_test_utils::telemetry::assert_declares;
    assert_declares(&capture.declared(), "store_refresh_token", "user_id");
    assert_declares(
        &capture.declared(),
        "get_session_by_refresh_token",
        "token_hash",
    );
    assert_declares(&capture.declared(), "revoke_session", "token_hash");

    // Negative space: raw token, stored digest, provenance, and the attacker-chosen
    // revocation sentinel appear nowhere — literal or percent-decoded.
    assert_absent_plain_and_encoded(&rendered, &raw_refresh_token);
    assert_absent_plain_and_encoded(&rendered, &token_hash_hex);
    assert_absent_plain_and_encoded(&rendered, hostile_token);
    for provenance in [PROVENANCE_IP, PROVENANCE_UA, PROVENANCE_DEVICE] {
        assert_absent_plain_and_encoded(&rendered, provenance);
    }
}

/// An unknown refresh grant is rejected with exactly the generic invalid_token body —
/// never an echo of the presented token — while the operator's warn diagnostic still
/// fires under the request span.
#[tokio::test]
async fn unknown_refresh_grant_is_generic_and_does_not_echo_the_token() {
    let corpus = build_corpus_app().await;
    let capture = install_span_capture(SharedBuffer::default());

    let presented = "PRESENTED-BUT-UNKNOWN-REFRESH-SENTINEL";
    let response = corpus
        .app
        .oneshot(form_request(
            "/token",
            format!("grant_type=refresh_token&refresh_token={presented}"),
        ))
        .await
        .expect("in-flight request");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = body_to_json(response.into_body()).await;
    assert_eq!(body["error"], "invalid_token");
    assert_eq!(
        body["error_description"],
        oidc_exchange_core::error::Error::InvalidToken {
            reason: String::new()
        }
        .client_description(),
        "the body must be exactly the variant's static description"
    );
    assert!(
        !body["error_description"]
            .as_str()
            .unwrap()
            .contains(presented),
        "the presented token must not be echoed to the caller"
    );

    let rendered = capture.rendered();
    assert!(
        rendered.contains("client fault mapped to error response"),
        "the warn diagnostic must land under the request span"
    );
    assert_absent_plain_and_encoded(&rendered, presented);
}
