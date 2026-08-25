use std::collections::HashMap;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use sha2::{Digest, Sha256};

use oidc_exchange_core::config::{
    Config, RawAuditConfig, RawConfig, RawRegistrationConfig, RawServerConfig, RawTelemetryConfig,
    RawTokenConfig,
};
use oidc_exchange_core::domain::{
    is_valid_family_id, AccessTokenClaims, AuditEventType, AuditFailure, AuditOutcome,
};
use oidc_exchange_core::error::Error;
use oidc_exchange_core::ports::{IdentityProvider, KeyManager, SessionRepository};
use oidc_exchange_core::service::exchange::{ExchangeCredential, ExchangeRequest};
use oidc_exchange_core::service::refresh::RefreshRequest;
use oidc_exchange_core::service::revoke::RevokeRequest;
use oidc_exchange_core::service::AppService;

use oidc_exchange_test_utils::{
    MockAuditLog, MockIdentityProvider, MockKeyManager, MockRateLimiter, MockRepository,
    MockUserSync,
};

fn base_raw_config() -> RawConfig {
    RawConfig {
        server: RawServerConfig {
            host: "127.0.0.1".to_string(),
            port: 8080,
            issuer: "https://auth.test.com".to_string(),
            role: "all".to_string(),
            request_timeout: "30s".to_string(),
            base_path: None,
            ..RawServerConfig::default()
        },
        registration: RawRegistrationConfig {
            mode: "open".to_string(),
            domain_allowlist: None,
        },
        token: RawTokenConfig {
            access_token_ttl: "15m".to_string(),
            refresh_token_ttl: "30d".to_string(),
            audience: "https://api.test.com".to_string(),
            custom_claims: None,
            ..RawTokenConfig::default()
        },
        audit: RawAuditConfig {
            adapter: "noop".to_string(),
            blocking_threshold: "warning".to_string(),
            emit_threshold: "info".to_string(),
            sqs: None,
            ..RawAuditConfig::default()
        },
        telemetry: RawTelemetryConfig {
            enabled: false,
            exporter: "none".to_string(),
            endpoint: None,
            service_name: None,
            sample_rate: None,
            protocol: None,
        },
        ..RawConfig::default()
    }
}

fn make_config() -> Config {
    Config::resolve(base_raw_config()).expect("test config should resolve")
}
fn make_service(repo: MockRepository, provider: MockIdentityProvider) -> AppService {
    let provider_id = provider.provider_id().to_string();
    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    providers.insert(provider_id, Box::new(provider));

    AppService::new(
        Box::new(repo.clone()),
        Box::new(repo),
        Box::new(MockKeyManager::new()),
        Box::new(MockAuditLog::new()),
        Box::new(MockUserSync::new()),
        Box::new(MockRateLimiter::new()),
        providers,
        make_config(),
    )
}

/// Builds a service whose `AuditLog` is a caller-supplied `MockAuditLog`, so
/// the test can inspect recorded events after the revoke call.
fn make_service_with_audit_and_config(
    repo: MockRepository,
    provider: MockIdentityProvider,
    audit: MockAuditLog,
    config: Config,
) -> AppService {
    let provider_id = provider.provider_id().to_string();
    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    providers.insert(provider_id, Box::new(provider));

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

fn make_service_with_audit(
    repo: MockRepository,
    provider: MockIdentityProvider,
    audit: MockAuditLog,
) -> AppService {
    make_service_with_audit_and_config(repo, provider, audit, make_config())
}

/// Helper: perform an exchange and return the full token response.
async fn do_exchange(svc: &AppService) -> oidc_exchange_core::domain::TokenResponse {
    let request = ExchangeRequest {
        provider_access_token: None,
        credential: ExchangeCredential::AuthorizationCode {
            code: "auth-code".to_string(),
            redirect_uri: "https://app.test.com/callback".to_string(),
        },
        provider: "mock".to_string(),
        ip_address: None,
        user_agent: None,
        device_id: None,
    };
    svc.exchange(request)
        .await
        .expect("exchange should succeed")
}

#[tokio::test]
async fn revoke_refresh_token_removes_session() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let svc = make_service(repo.clone(), provider);

    // Exchange to get tokens
    let response = do_exchange(&svc).await;
    let refresh_token = response.refresh_token.expect("should have refresh token").into_inner();

    // Verify session exists
    let sessions = repo.get_all_sessions().await;
    assert_eq!(sessions.len(), 1);

    // Revoke the refresh token
    let revoke_req = RevokeRequest {
        token: refresh_token.clone(),
        token_type_hint: Some("refresh_token".to_string()),
        ..Default::default()
    };
    svc.revoke(revoke_req).await.expect("revoke should succeed");

    // Verify session is removed
    let sessions = repo.get_all_sessions().await;
    assert_eq!(
        sessions.len(),
        0,
        "session should be removed after revocation"
    );

    // Also verify by hash lookup
    let token_hash = hex::encode(Sha256::digest(refresh_token.as_bytes()));
    let session = repo
        .get_session_by_refresh_token(&oidc_exchange_core::Secret::new(token_hash.clone()))
        .await
        .expect("lookup should not error");
    assert!(
        session.is_none(),
        "session should not exist after revocation"
    );
}

#[tokio::test]
async fn revoke_access_token_revokes_its_family_live_and_retired() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let svc = make_service(repo.clone(), provider);

    // Two independent sign-ins → two families for the same user.
    let response1 = do_exchange(&svc).await;
    let response2 = do_exchange(&svc).await;

    // Rotate family 1 once so it holds a live generation AND a retained
    // retirement record at revocation time.
    let presented1 = response1
        .refresh_token
        .clone()
        .expect("exchange issues a token");
    let _gen1 = svc
        .refresh(RefreshRequest {
            refresh_token: presented1.expose().clone(),
            ..Default::default()
        })
        .await
        .expect("rotation should succeed")
        .refresh_token
        .expect("rotation issues a replacement")
        .into_inner();
    assert_eq!(repo.get_all_sessions().await.len(), 2);
    assert_eq!(repo.get_all_retired_tokens().await.len(), 1);

    // The first exchange's access token names family 1, well-formed.
    let parts: Vec<&str> = response1.access_token.split('.').collect();
    let payload_bytes = URL_SAFE_NO_PAD
        .decode(parts[1])
        .expect("payload should be valid base64url");
    let claims: AccessTokenClaims =
        serde_json::from_slice(&payload_bytes).expect("payload should deserialize");
    assert!(
        is_valid_family_id(&claims.sid),
        "issued access tokens carry a well-formed family sid, got {:?}",
        claims.sid
    );

    // Revoke with that access token.
    let revoke_req = RevokeRequest {
        token: response1.access_token.clone(),
        token_type_hint: Some("access_token".to_string()),
        ..Default::default()
    };
    svc.revoke(revoke_req).await.expect("revoke should succeed");

    // Family 1 is gone entirely: its live generation and its retirement
    // records. Family 2 keeps exactly its own session.
    let remaining = repo.get_all_sessions().await;
    assert_eq!(remaining.len(), 1, "only the sibling family survives");
    assert_ne!(
        remaining[0].family_id, claims.sid,
        "the surviving session must belong to the other family"
    );
    assert!(
        repo.get_all_retired_tokens().await.is_empty(),
        "revocation must remove the family's retirement records too"
    );
    let retired_hash = hex::encode(Sha256::digest(presented1.expose().as_bytes()));
    let resolution = repo
        .resolve_refresh_token(&retired_hash)
        .await
        .expect("classify revoked generation");
    assert!(
        matches!(
            resolution,
            oidc_exchange_core::domain::RefreshResolution::Unknown
        ),
        "revocation removes retirement records, not just the live row"
    );

    // The subject's other family still redeems: no user-wide revocation.
    svc.refresh(RefreshRequest {
        refresh_token: response2.refresh_token.expect("sibling exchange token").into_inner(),
        ..Default::default()
    })
    .await
    .expect("the sibling family must be unaffected");
}

#[tokio::test]
async fn revoke_unknown_token_returns_ok() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let svc = make_service(repo.clone(), provider);

    // Revoke a random token that was never issued — should not error per RFC 7009
    let revoke_req = RevokeRequest {
        token: "this-token-does-not-exist-at-all".to_string(),
        token_type_hint: Some("refresh_token".to_string()),
        ..Default::default()
    };
    let result = svc.revoke(revoke_req).await;
    assert!(
        result.is_ok(),
        "revoke should always return Ok per RFC 7009"
    );

    // Also try with access_token hint and a bogus JWT
    let revoke_req = RevokeRequest {
        token: "not.a.valid-jwt".to_string(),
        token_type_hint: Some("access_token".to_string()),
        ..Default::default()
    };
    let result = svc.revoke(revoke_req).await;
    assert!(
        result.is_ok(),
        "revoke should always return Ok per RFC 7009"
    );

    // Also try with a completely garbage string (not even JWT-shaped)
    let revoke_req = RevokeRequest {
        token: "garbage".to_string(),
        token_type_hint: Some("access_token".to_string()),
        ..Default::default()
    };
    let result = svc.revoke(revoke_req).await;
    assert!(
        result.is_ok(),
        "revoke should always return Ok per RFC 7009"
    );
}

#[tokio::test]
async fn revoke_default_hint_treats_as_refresh_token() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let svc = make_service(repo.clone(), provider);

    // Exchange to get tokens
    let response = do_exchange(&svc).await;
    let refresh_token = response.refresh_token.expect("should have refresh token").into_inner();

    // Verify session exists
    let sessions = repo.get_all_sessions().await;
    assert_eq!(sessions.len(), 1);

    // Revoke with token_type_hint = None (should default to refresh_token behavior)
    let revoke_req = RevokeRequest {
        token: refresh_token.clone(),
        token_type_hint: None,
        ..Default::default()
    };
    svc.revoke(revoke_req).await.expect("revoke should succeed");

    // Verify session is removed (proving it was treated as a refresh token)
    let sessions = repo.get_all_sessions().await;
    assert_eq!(
        sessions.len(),
        0,
        "session should be removed when hint is None (defaults to refresh_token)"
    );
}

#[tokio::test]
async fn revoke_forged_access_token_does_not_revoke_sessions() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let audit = MockAuditLog::new();
    let audit_clone = audit.clone();
    let svc = make_service_with_audit(repo.clone(), provider, audit);

    // Exchange to create a session
    let _response = do_exchange(&svc).await;
    let sessions = repo.get_all_sessions().await;
    assert_eq!(sessions.len(), 1);
    // `do_exchange` itself emits UserCreated/TokenExchange events; capture
    // the baseline so the assertions below are scoped to what `revoke` adds.
    let baseline = audit_clone.events().await.len();

    // Craft a well-shaped access token (correct header, full claims) with an
    // invalid signature, so the rejection is attributable to verification.
    let now = chrono::Utc::now().timestamp();
    let header_json = serde_json::json!({
        "alg": "EdDSA",
        "typ": "at+jwt",
        "kid": "test-key-1"
    });
    let payload_json = serde_json::json!({
        "sub": sessions[0].user_id,
        "iss": "https://auth.test.com",
        "aud": "https://api.test.com",
        "iat": now,
        "exp": now + 900,
        "sid": sessions[0].refresh_token_hash,
    });
    let forged_header =
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&header_json).expect("header serializes"));
    let forged_payload =
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload_json).expect("payload serializes"));
    let signing_input = format!("{forged_header}.{forged_payload}");
    let forged_sig = URL_SAFE_NO_PAD.encode([7u8; 64]); // bogus signature
    let forged_jwt = format!("{}.{}", signing_input, forged_sig);

    // Revoke with the forged JWT: Ok toward the client, one failure event for
    // operators, and no session mutation.
    let revoke_req = RevokeRequest {
        token: forged_jwt,
        token_type_hint: Some("access_token".to_string()),
        ..Default::default()
    };
    let result = svc.revoke(revoke_req).await;
    assert!(result.is_ok(), "revoke should return Ok per RFC 7009");

    let events = audit_clone.events().await;
    assert_eq!(
        events.len(),
        baseline + 1,
        "a forged access token must emit exactly one failure event"
    );
    assert_eq!(
        events
            .last()
            .expect("an event was just recorded")
            .event_type,
        AuditEventType::ValidationFailed
    );

    // Sessions should NOT be revoked because the JWT signature is invalid
    let sessions_after = repo.get_all_sessions().await;
    assert_eq!(
        sessions_after.len(),
        1,
        "sessions should NOT be revoked for a forged access token"
    );
}

#[tokio::test]
async fn revoke_enforce_audit_failure_is_indistinguishable_for_existing_and_unknown_refresh_tokens()
{
    let audit = MockAuditLog::new();
    let audit_clone = audit.clone();
    let config = {
        let mut raw = base_raw_config();
        raw.audit.durability = "enforce".to_string();
        Config::resolve(raw).expect("enforce config resolves")
    };
    let repo = MockRepository::new();
    let svc = make_service_with_audit_and_config(
        repo.clone(),
        MockIdentityProvider::new("mock"),
        audit,
        config,
    );

    let existing_token = do_exchange(&svc)
        .await
        .refresh_token
        .expect("exchange returns refresh token")
        .into_inner();
    audit_clone.set_fail_mode(true).await;

    let existing = svc
        .revoke(RevokeRequest {
            token: existing_token,
            token_type_hint: Some("refresh_token".to_string()),
            ..Default::default()
        })
        .await;
    let unknown = svc
        .revoke(RevokeRequest {
            token: "never-issued-refresh-token".to_string(),
            token_type_hint: Some("refresh_token".to_string()),
            ..Default::default()
        })
        .await;

    assert!(matches!(
        existing,
        Err(oidc_exchange_core::error::Error::SecurityAuditDurability { .. })
    ));
    assert!(matches!(
        unknown,
        Err(oidc_exchange_core::error::Error::SecurityAuditDurability { .. })
    ));
    assert!(
        repo.get_all_sessions().await.is_empty(),
        "existing session is still revoked"
    );
}

#[tokio::test]
async fn revoke_valid_access_token_emits_token_revocation_with_family_count() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let audit = MockAuditLog::new();
    let audit_clone = audit.clone();
    let svc = make_service_with_audit(repo.clone(), provider, audit);

    let response = do_exchange(&svc).await;
    let user_id = repo.get_all_sessions().await[0].user_id.clone();
    // The token's sid doubles as the expected detail value.
    let parts: Vec<&str> = response.access_token.split('.').collect();
    let payload_bytes = URL_SAFE_NO_PAD
        .decode(parts[1])
        .expect("payload should be valid base64url");
    let claims: AccessTokenClaims =
        serde_json::from_slice(&payload_bytes).expect("payload should deserialize");
    // `do_exchange` itself emits UserCreated/TokenExchange events; capture
    // the baseline so the assertion below is scoped to what `revoke` adds.
    let baseline = audit_clone.events().await.len();

    let revoke_req = RevokeRequest {
        token: response.access_token.clone(),
        token_type_hint: Some("access_token".to_string()),
        ip_address: Some("203.0.113.5".to_string()),
        user_agent: Some("test-agent/1.0".to_string()),
        ..Default::default()
    };
    svc.revoke(revoke_req).await.expect("revoke should succeed");

    let events = audit_clone.events().await;
    assert_eq!(
        events.len(),
        baseline + 1,
        "a verified access-token revoke must emit exactly one audit event"
    );
    let revoke_event = events.last().expect("an event was just recorded");
    assert_eq!(revoke_event.event_type, AuditEventType::TokenRevocation);
    assert_eq!(revoke_event.outcome, AuditOutcome::Success);
    assert_eq!(revoke_event.actor, Some(user_id));
    assert_eq!(
        revoke_event.detail.get("family_id"),
        Some(&serde_json::json!(claims.sid)),
        "detail names exactly the family the token may revoke"
    );
    assert_eq!(
        revoke_event.detail.get("sessions_revoked"),
        Some(&serde_json::json!(1)),
        "one exchange → one live generation removed"
    );
    assert_eq!(revoke_event.ip_address, Some("203.0.113.5".to_string()));
    assert_eq!(revoke_event.user_agent, Some("test-agent/1.0".to_string()));
}

#[tokio::test]
async fn revoke_valid_refresh_token_emits_token_revocation() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let audit = MockAuditLog::new();
    let audit_clone = audit.clone();
    let svc = make_service_with_audit(repo.clone(), provider, audit);

    let response = do_exchange(&svc).await;
    let refresh_token = response.refresh_token.expect("should have refresh token").into_inner();
    let user_id = repo.get_all_sessions().await[0].user_id.clone();
    // `do_exchange` itself emits UserCreated/TokenExchange events; capture
    // the baseline so the assertion below is scoped to what `revoke` adds.
    let baseline = audit_clone.events().await.len();

    let revoke_req = RevokeRequest {
        token: refresh_token,
        token_type_hint: Some("refresh_token".to_string()),
        ip_address: Some("198.51.100.7".to_string()),
        user_agent: Some("test-agent/2.0".to_string()),
        ..Default::default()
    };
    svc.revoke(revoke_req).await.expect("revoke should succeed");

    let events = audit_clone.events().await;
    assert_eq!(
        events.len(),
        baseline + 1,
        "a valid refresh-token revoke must emit exactly one audit event"
    );
    let revoke_event = events.last().expect("an event was just recorded");
    assert_eq!(revoke_event.event_type, AuditEventType::TokenRevocation);
    assert_eq!(revoke_event.outcome, AuditOutcome::Success);
    assert_eq!(revoke_event.actor, Some(user_id));
    assert_eq!(revoke_event.ip_address, Some("198.51.100.7".to_string()));
    assert_eq!(revoke_event.user_agent, Some("test-agent/2.0".to_string()));
}

/// Sign arbitrary header/payload JSON with the same deterministic key the
/// service verifies against, so each mutant below isolates exactly one
/// validator check.
async fn signed_jwt(header: &serde_json::Value, payload: &serde_json::Value) -> String {
    let keys = MockKeyManager::new();
    let header_b64 = URL_SAFE_NO_PAD.encode(serde_json::to_vec(header).expect("header serializes"));
    let payload_b64 =
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(payload).expect("payload serializes"));
    let signing_input = format!("{}.{}", header_b64, payload_b64);
    let signature = keys
        .sign(signing_input.as_bytes())
        .await
        .expect("mock signing succeeds");
    format!("{}.{}", signing_input, URL_SAFE_NO_PAD.encode(signature))
}

#[tokio::test]
async fn revoke_claim_and_header_negatives_revoke_nothing_and_emit_one_failure_each() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let audit = MockAuditLog::new();
    let audit_clone = audit.clone();
    let svc = make_service_with_audit(repo.clone(), provider, audit);

    // One live session every mutant must leave untouched.
    let _response = do_exchange(&svc).await;
    let sessions_before = repo.get_all_sessions().await;
    assert_eq!(sessions_before.len(), 1);
    let stored_hash = sessions_before[0].refresh_token_hash.clone();
    // `do_exchange` itself emits UserCreated/TokenExchange events; capture
    // the baseline so event counts below are scoped to what `revoke` adds.
    let baseline = audit_clone.events().await.len();

    let now = chrono::Utc::now().timestamp();
    let good_header: serde_json::Value =
        serde_json::json!({"alg": "EdDSA", "typ": "at+jwt", "kid": "test-key-1"});
    let good_claims = || {
        serde_json::json!({
            "sub": sessions_before[0].user_id,
            "iss": "https://auth.test.com",
            "aud": "https://api.test.com",
            "iat": now,
            "exp": now + 900,
            "sid": stored_hash,
        })
    };

    // (label, header, payload): the source-spec claim/header negatives.
    let mut expired = good_claims();
    expired["exp"] = serde_json::json!((now - 61) as u64); // one second past the 60s skew
    let mut future_nbf = good_claims();
    future_nbf["nbf"] = serde_json::json!((now + 61) as u64);
    let mut wrong_iss = good_claims();
    wrong_iss["iss"] = serde_json::json!("https://evil.example.com");
    let mut wrong_aud = good_claims();
    wrong_aud["aud"] = serde_json::json!("https://other-api.example.com");
    let mut missing_exp = good_claims();
    missing_exp.as_object_mut().unwrap().remove("exp");
    let mut missing_sid = good_claims();
    missing_sid.as_object_mut().unwrap().remove("sid");

    let cases: Vec<(&str, serde_json::Value, serde_json::Value)> = vec![
        ("expired exp", good_header.clone(), expired),
        ("future nbf", good_header.clone(), future_nbf),
        ("wrong iss", good_header.clone(), wrong_iss),
        ("wrong aud", good_header.clone(), wrong_aud),
        (
            "wrong alg",
            serde_json::json!({"alg": "HS256", "typ": "at+jwt", "kid": "test-key-1"}),
            good_claims(),
        ),
        (
            "wrong typ",
            serde_json::json!({"alg": "EdDSA", "typ": "JWT", "kid": "test-key-1"}),
            good_claims(),
        ),
        ("missing exp", good_header.clone(), missing_exp),
        ("missing sid", good_header.clone(), missing_sid),
    ];

    for (index, (label, header, payload)) in cases.iter().enumerate() {
        let token = signed_jwt(header, payload).await;
        let revoke_req = RevokeRequest {
            token,
            token_type_hint: Some("access_token".to_string()),
            ip_address: Some("203.0.113.9".to_string()),
            user_agent: Some("test-agent/3.0".to_string()),
            ..Default::default()
        };
        let result = svc.revoke(revoke_req).await;
        assert!(
            result.is_ok(),
            "case {label}: revoke must return Ok per RFC 7009"
        );

        // Exactly one failure event per rejection, in submission order.
        let events = audit_clone.events().await;
        assert_eq!(
            events.len(),
            baseline + index + 1,
            "case {label}: exactly one ValidationFailed must be recorded"
        );
        let event = events.last().expect("an event was just recorded");
        assert_eq!(
            event.event_type,
            AuditEventType::ValidationFailed,
            "case {label}"
        );
        assert_eq!(
            event.outcome,
            AuditOutcome::Failure(AuditFailure::AuthenticationFailed),
            "case {label}: the rejection must carry the fixed failure class"
        );

        // And the session store is untouched by every rejected credential.
        assert_eq!(
            repo.get_all_sessions().await.len(),
            1,
            "case {label}: the live session must survive"
        );
    }
}

#[tokio::test]
async fn revoke_access_token_store_failures_propagate() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let svc = make_service(repo.clone(), provider);

    let response = do_exchange(&svc).await;

    // Failure of the mutating revoke call propagates as an error.
    repo.set_session_fail_mode(true).await;
    let revoke_req = RevokeRequest {
        token: response.access_token.clone(),
        token_type_hint: Some("access_token".to_string()),
        ..Default::default()
    };
    let err = svc
        .revoke(revoke_req)
        .await
        .expect_err("a session-store failure on the mutating path must propagate");

    // The family revocation path performs no presence-check lookup: the
    // validated token itself names the family, so only the mutating call can
    // fail. (The refresh-token arm still exercises the lookup path.)
    assert!(
        matches!(err, Error::StoreError { .. }),
        "expected StoreError from the mutating path, got {err:?}"
    );

    // With the store healthy again the same token revokes normally.
    repo.set_session_fail_mode(false).await;
    let revoke_req = RevokeRequest {
        token: response.access_token,
        token_type_hint: Some("access_token".to_string()),
        ..Default::default()
    };
    svc.revoke(revoke_req)
        .await
        .expect("a healthy store revokes without error");
    assert_eq!(
        repo.get_all_sessions().await.len(),
        0,
        "the sid-named session should be gone after recovery"
    );
}

#[tokio::test]
async fn revoke_unknown_refresh_token_emits_authentication_failure() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let audit = MockAuditLog::new();
    let audit_clone = audit.clone();
    let svc = make_service_with_audit(repo.clone(), provider, audit);

    let revoke_req = RevokeRequest {
        token: "this-refresh-token-was-never-issued".to_string(),
        token_type_hint: Some("refresh_token".to_string()),
        ip_address: Some("203.0.113.11".to_string()),
        user_agent: Some("test-agent/4.0".to_string()),
        ..Default::default()
    };
    let result = svc.revoke(revoke_req).await;
    assert!(
        result.is_ok(),
        "revoke should always return Ok per RFC 7009"
    );

    let events = audit_clone.events().await;
    assert_eq!(
        events.len(),
        1,
        "an unknown refresh token must emit exactly one terminal audit event"
    );
    let event = events.last().expect("a terminal event was just recorded");
    assert_eq!(event.event_type, AuditEventType::ValidationFailed);
    assert_eq!(
        event.outcome,
        AuditOutcome::Failure(oidc_exchange_core::domain::AuditFailure::AuthenticationFailed)
    );
}

// ===========================================================================
// Task 08 — the sid seam's fail-closed consumption boundary
//
// A validly-SIGNED token whose `sid` cannot name a family (hash-form
// pre-rotation mint, blank, malformed, or missing entirely) must revoke
// nothing and emit exactly one fixed-reason rejection, while a forged
// signature stays fully silent per RFC 7009. The signed-JWT helper makes
// these tokens without reconstructing signing keys.
// ===========================================================================

/// The 64-hex SHA-256 form pre-rotation access tokens carried in `sid`.
const LEGACY_HASH_SID: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// Build a service whose audit log records even Debug-severity rejections.
fn make_debug_audit_service(
    repo: MockRepository,
    provider: MockIdentityProvider,
) -> (AppService, MockAuditLog) {
    let config = {
        let mut raw = base_raw_config();
        raw.audit.emit_threshold = "debug".to_string();
        Config::resolve(raw).expect("debug audit config resolves")
    };
    let provider_id = provider.provider_id().to_string();
    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    providers.insert(provider_id, Box::new(provider));

    let audit = MockAuditLog::new();
    let svc = AppService::new(
        Box::new(repo.clone()),
        Box::new(repo),
        Box::new(MockKeyManager::new()),
        Box::new(audit.clone()),
        Box::new(MockUserSync::new()),
        Box::new(oidc_exchange_test_utils::MockRateLimiter::new()),
        providers,
        config,
    );
    (svc, audit)
}

/// Signed claims with an attacker-chosen (or legacy) `sid`.
fn signed_claims(sid: Option<&str>) -> serde_json::Value {
    let mut payload = serde_json::json!({
        "sub": "usr_someone",
        "iss": "https://auth.test.com",
        "aud": "",
        "iat": 0,
        "exp": u64::MAX / 2,
    });
    if let Some(sid) = sid {
        payload["sid"] = serde_json::Value::from(sid);
    }
    payload
}

/// Every unusable-sid shape — hash-form legacy, blank, malformed, and a
/// payload with no `sid` at all — fails closed identically: Ok per RFC 7009,
/// nothing revoked, one fixed-reason ValidationFailed each.
#[tokio::test]
async fn unusable_sids_fail_closed_before_any_mutation() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let (svc, audit) = make_debug_audit_service(repo.clone(), provider);

    // Seed one real session to prove no presentation mutates the store.
    do_exchange(&svc).await;
    let sessions_before = repo.get_all_sessions().await;
    assert_eq!(sessions_before.len(), 1);

    for sid in [
        Some(LEGACY_HASH_SID),
        Some(""),
        Some("not-a-family"),
        Some("fam_short"),
        None,
    ] {
        let jwt = MockKeyManager::new().sign_payload_jws(&signed_claims(sid));
        svc.revoke(RevokeRequest {
            token: jwt,
            token_type_hint: Some("access_token".to_string()),
            ip_address: Some("203.0.113.41".to_string()),
            ..Default::default()
        })
        .await
        .expect("fail-closed revocation stays Ok per RFC 7009");
    }

    // The store is byte-identical: no family was ever touched.
    assert_eq!(
        repo.get_all_sessions().await,
        sessions_before,
        "unusable sids must revoke nothing"
    );
    assert!(repo.get_all_retired_tokens().await.is_empty());

    // Exactly one fixed-reason rejection per presented token.
    let events = audit.events().await;
    let baseline = 2; // exchange-era UserCreated + TokenExchange
    assert_eq!(events.len(), baseline + 5);
    let rejections: Vec<_> = events[baseline..]
        .iter()
        .map(|e| (e.event_type.clone(), e.outcome.clone()))
        .collect();
    for (event_type, outcome) in &rejections {
        assert_eq!(*event_type, AuditEventType::ValidationFailed);
        // The closed failure classification can never echo the offending sid
        // value — every unusable-sid rejection carries the identical class.
        assert_eq!(
            *outcome,
            AuditOutcome::Failure(AuditFailure::AuthenticationFailed),
            "every unusable-sid rejection carries the same fixed classification"
        );
    }
}

/// A forged signature succeeds toward the client (RFC 7009) while emitting
/// exactly one fixed-reason `ValidationFailed` — the first-party validator's
/// rejection contract: the attempt stays visible to operators, and the fixed
/// reason never echoes token bytes.
#[tokio::test]
async fn forged_signature_stays_silent_to_the_client_but_audits_once() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let (svc, audit) = make_debug_audit_service(repo, provider);

    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"EdDSA","typ":"JWT"}"#);
    let payload =
        URL_SAFE_NO_PAD.encode(signed_claims(Some(LEGACY_HASH_SID)).to_string().as_bytes());
    let forged = format!("{header}.{payload}.{}", URL_SAFE_NO_PAD.encode([0u8; 64]));

    svc.revoke(RevokeRequest {
        token: forged,
        token_type_hint: Some("access_token".to_string()),
        ..Default::default()
    })
    .await
    .expect("forged revocation stays Ok per RFC 7009");

    let events = audit.events().await;
    assert_eq!(
        events.len(),
        1,
        "a rejected credential emits exactly one audit event, got {:?}",
        events
            .iter()
            .map(|e| e.event_type.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(events[0].event_type, AuditEventType::ValidationFailed);
    assert!(matches!(events[0].outcome, AuditOutcome::Failure { .. }));
}

/// Issued tokens' sid is stable across rotation: exchange and its successor
/// refresh carry the identical well-formed family identifier.
#[tokio::test]
async fn sid_is_invariant_across_rotation() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let svc = make_service(repo.clone(), provider);

    let exchange_response = do_exchange(&svc).await;
    let issued_sid = {
        let parts: Vec<&str> = exchange_response.access_token.split('.').collect();
        let payload = URL_SAFE_NO_PAD.decode(parts[1]).expect("b64url payload");
        let claims: AccessTokenClaims = serde_json::from_slice(&payload).expect("typed claims");
        claims.sid
    };
    assert!(is_valid_family_id(&issued_sid));
    assert_eq!(
        issued_sid,
        repo.get_all_sessions().await[0].family_id,
        "the sid names exactly the session family exchange created"
    );

    let refresh_response = svc
        .refresh(RefreshRequest {
            refresh_token: exchange_response
                .refresh_token
                .expect("exchange issues a token")
                .into_inner(),
            ..Default::default()
        })
        .await
        .expect("rotation succeeds");
    let rotated_sid = {
        let parts: Vec<&str> = refresh_response.access_token.split('.').collect();
        let payload = URL_SAFE_NO_PAD.decode(parts[1]).expect("b64url payload");
        let claims: AccessTokenClaims = serde_json::from_slice(&payload).expect("typed claims");
        claims.sid
    };
    assert_eq!(
        issued_sid, rotated_sid,
        "rotation must not move the sid claim"
    );
}
