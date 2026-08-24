use std::collections::HashMap;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use sha2::{Digest, Sha256};

use oidc_exchange_core::config::{
    Config, RawAuditConfig, RawConfig, RawRegistrationConfig, RawServerConfig, RawTelemetryConfig,
    RawTokenConfig,
};
use oidc_exchange_core::domain::{AccessTokenClaims, AuditEventType, AuditOutcome};
use oidc_exchange_core::error::Error;
use oidc_exchange_core::ports::{IdentityProvider, KeyManager, SessionRepository};
use oidc_exchange_core::service::exchange::ExchangeRequest;
use oidc_exchange_core::service::revoke::RevokeRequest;
use oidc_exchange_core::service::AppService;

use oidc_exchange_test_utils::{
    MockAuditLog, MockIdentityProvider, MockKeyManager, MockRepository, MockUserSync,
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
        },
        audit: RawAuditConfig {
            adapter: "noop".to_string(),
            blocking_threshold: "warning".to_string(),
            emit_threshold: "info".to_string(),
            sqs: None,
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
        providers,
        make_config(),
    )
}

/// Builds a service whose `AuditLog` is a caller-supplied `MockAuditLog`, so
/// the test can inspect recorded events after the revoke call.
fn make_service_with_audit(
    repo: MockRepository,
    provider: MockIdentityProvider,
    audit: MockAuditLog,
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
        providers,
        make_config(),
    )
}

/// Helper: perform an exchange and return the full token response.
async fn do_exchange(svc: &AppService) -> oidc_exchange_core::domain::TokenResponse {
    let request = ExchangeRequest {
        code: Some("auth-code".to_string()),
        redirect_uri: Some("https://app.test.com/callback".to_string()),
        id_token: None,
        provider: "mock".to_string(),
        ..Default::default()
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
    let refresh_token = response.refresh_token.expect("should have refresh token");

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
        .get_session_by_refresh_token(&token_hash)
        .await
        .expect("lookup should not error");
    assert!(
        session.is_none(),
        "session should not exist after revocation"
    );
}

#[tokio::test]
async fn revoke_access_token_removes_only_the_session_its_sid_names() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let svc = make_service(repo.clone(), provider);

    // Exchange twice to create two sessions for the same user.
    let response1 = do_exchange(&svc).await;
    let response2 = do_exchange(&svc).await;
    let refresh1 = response1
        .refresh_token
        .expect("first exchange returns refresh token");
    let refresh2 = response2
        .refresh_token
        .expect("second exchange returns refresh token");

    // Verify two same-user sessions exist.
    let sessions = repo.get_all_sessions().await;
    assert_eq!(sessions.len(), 2, "should have two sessions");
    assert_eq!(
        sessions[0].user_id, sessions[1].user_id,
        "both sessions should belong to same user"
    );

    // The presented access token names exactly its own session.
    let hash1 = hex::encode(Sha256::digest(refresh1.as_bytes()));
    let parts: Vec<&str> = response1.access_token.split('.').collect();
    let payload_bytes = URL_SAFE_NO_PAD
        .decode(parts[1])
        .expect("payload should be valid base64url");
    let claims: AccessTokenClaims =
        serde_json::from_slice(&payload_bytes).expect("payload should deserialize");
    assert_eq!(
        claims.sid, hash1,
        "the token's sid must be its own session's refresh-token hash"
    );

    // Revoke using the first exchange's access token.
    let revoke_req = RevokeRequest {
        token: response1.access_token.clone(),
        token_type_hint: Some("access_token".to_string()),
        ..Default::default()
    };
    svc.revoke(revoke_req).await.expect("revoke should succeed");

    // Only the named session is gone; the sibling stays live.
    let sessions = repo.get_all_sessions().await;
    assert_eq!(
        sessions.len(),
        1,
        "exactly one session should survive an access-token revocation"
    );
    let hash2 = hex::encode(Sha256::digest(refresh2.as_bytes()));
    assert_eq!(
        sessions[0].refresh_token_hash, hash2,
        "the surviving session must be the sibling, not the revoked one"
    );
    // And the revoked hash no longer resolves.
    let revoked = repo
        .get_session_by_refresh_token(&hash1)
        .await
        .expect("lookup should not error");
    assert!(revoked.is_none(), "the sid-named session must be revoked");
}

#[tokio::test]
async fn revoke_valid_access_token_emits_token_revocation() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let audit = MockAuditLog::new();
    let audit_clone = audit.clone();
    let svc = make_service_with_audit(repo.clone(), provider, audit);

    let response = do_exchange(&svc).await;
    let user_id = repo.get_all_sessions().await[0].user_id.clone();
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
        "a valid access-token revoke must emit exactly one audit event"
    );
    let revoke_event = events.last().expect("an event was just recorded");
    assert_eq!(revoke_event.event_type, AuditEventType::TokenRevocation);
    assert_eq!(revoke_event.outcome, AuditOutcome::Success);
    // The actor is the stored session's user, not a claim read off the token.
    assert_eq!(revoke_event.actor, Some(user_id));
    assert_eq!(revoke_event.ip_address, Some("203.0.113.5".to_string()));
    assert_eq!(revoke_event.user_agent, Some("test-agent/1.0".to_string()));
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
    let refresh_token = response.refresh_token.expect("should have refresh token");

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
async fn revoke_valid_refresh_token_emits_token_revocation() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let audit = MockAuditLog::new();
    let audit_clone = audit.clone();
    let svc = make_service_with_audit(repo.clone(), provider, audit);

    let response = do_exchange(&svc).await;
    let refresh_token = response.refresh_token.expect("should have refresh token");
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
        match &event.outcome {
            AuditOutcome::Failure { reason } => assert!(
                !reason.is_empty(),
                "case {label}: the fixed reason must be non-empty"
            ),
            other => panic!("case {label}: expected a Failure outcome, got {other:?}"),
        }

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

    // Failure of the presence-check lookup propagates too.
    repo.set_session_fail_mode(false).await;
    repo.set_session_lookup_fail_mode(true).await;
    let revoke_req = RevokeRequest {
        token: response.access_token.clone(),
        token_type_hint: Some("access_token".to_string()),
        ..Default::default()
    };
    let err_lookup = svc
        .revoke(revoke_req)
        .await
        .expect_err("a session-store failure on the lookup path must propagate");

    match (err, err_lookup) {
        (Error::StoreError { .. }, Error::StoreError { .. }) => {}
        (other_a, other_b) => {
            panic!("expected StoreError from both paths, got {other_a:?} and {other_b:?}")
        }
    }

    // With the store healthy again the same token revokes normally.
    repo.set_session_lookup_fail_mode(false).await;
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
async fn revoke_unknown_refresh_token_emits_nothing() {
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
        0,
        "an unknown refresh token must not emit any audit event"
    );
}
