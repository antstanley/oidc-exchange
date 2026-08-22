use std::collections::HashMap;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use sha2::{Digest, Sha256};

use oidc_exchange_core::config::{AppConfig, AuditConfig, ServerConfig, TokenConfig};
use oidc_exchange_core::domain::{
    is_valid_family_id, AccessTokenClaims, AuditEventType, AuditOutcome,
};
use oidc_exchange_core::ports::{IdentityProvider, SessionRepository};
use oidc_exchange_core::service::exchange::ExchangeRequest;
use oidc_exchange_core::service::refresh::RefreshRequest;
use oidc_exchange_core::service::revoke::RevokeRequest;
use oidc_exchange_core::service::AppService;

use oidc_exchange_test_utils::{
    MockAuditLog, MockIdentityProvider, MockKeyManager, MockRepository, MockUserSync,
};

fn make_config() -> AppConfig {
    AppConfig {
        server: ServerConfig {
            issuer: "https://auth.test.com".to_string(),
            ..Default::default()
        },
        token: TokenConfig {
            access_token_ttl: "15m".to_string(),
            refresh_token_ttl: "30d".to_string(),
            audience: Some("https://api.test.com".to_string()),
            ..Default::default()
        },
        ..Default::default()
    }
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
            refresh_token: presented1.clone(),
            ..Default::default()
        })
        .await
        .expect("rotation should succeed")
        .refresh_token
        .expect("rotation issues a replacement");
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
    let retired_hash = hex::encode(Sha256::digest(presented1.as_bytes()));
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
        refresh_token: response2.refresh_token.expect("sibling exchange token"),
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
    let svc = make_service(repo.clone(), provider);

    // Exchange to create a session
    let _response = do_exchange(&svc).await;
    let sessions = repo.get_all_sessions().await;
    assert_eq!(sessions.len(), 1);
    let user_id = sessions[0].user_id.clone();

    // Craft a forged JWT with the real user's sub but an invalid signature
    let forged_header = URL_SAFE_NO_PAD.encode(br#"{"alg":"EdDSA","typ":"JWT"}"#);
    let forged_payload = URL_SAFE_NO_PAD.encode(
        format!(r#"{{"sub":"{user_id}","iss":"https://auth.test.com","iat":0,"exp":9999999999}}"#)
            .as_bytes(),
    );
    let forged_sig = URL_SAFE_NO_PAD.encode([0u8; 64]); // bogus signature
    let forged_jwt = format!("{forged_header}.{forged_payload}.{forged_sig}");

    // Revoke with the forged JWT
    let revoke_req = RevokeRequest {
        token: forged_jwt,
        token_type_hint: Some("access_token".to_string()),
        ..Default::default()
    };
    let result = svc.revoke(revoke_req).await;
    assert!(result.is_ok(), "revoke should return Ok per RFC 7009");

    // Sessions should NOT be revoked because the JWT signature is invalid
    let sessions = repo.get_all_sessions().await;
    assert_eq!(
        sessions.len(),
        1,
        "sessions should NOT be revoked for a forged access token"
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

#[tokio::test]
async fn revoke_failed_verification_access_token_emits_nothing() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let audit = MockAuditLog::new();
    let audit_clone = audit.clone();
    let svc = make_service_with_audit(repo.clone(), provider, audit);

    // Create a real session so we can also assert it survives untouched.
    let _response = do_exchange(&svc).await;
    let sessions_before = repo.get_all_sessions().await;
    assert_eq!(sessions_before.len(), 1);
    let user_id = sessions_before[0].user_id.clone();
    // `do_exchange` itself emits UserCreated/TokenExchange events; capture
    // the baseline so the assertion below is scoped to what `revoke` adds.
    let baseline = audit_clone.events().await.len();

    // Forge a JWT with a valid shape but an invalid signature.
    let forged_header = URL_SAFE_NO_PAD.encode(br#"{"alg":"EdDSA","typ":"JWT"}"#);
    let forged_payload = URL_SAFE_NO_PAD.encode(
        format!(r#"{{"sub":"{user_id}","iss":"https://auth.test.com","iat":0,"exp":9999999999}}"#)
            .as_bytes(),
    );
    let forged_sig = URL_SAFE_NO_PAD.encode([0u8; 64]);
    let forged_jwt = format!("{forged_header}.{forged_payload}.{forged_sig}");

    let revoke_req = RevokeRequest {
        token: forged_jwt,
        token_type_hint: Some("access_token".to_string()),
        ip_address: Some("203.0.113.9".to_string()),
        user_agent: Some("test-agent/3.0".to_string()),
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
        baseline,
        "failed signature verification must not emit any audit event"
    );
    assert_eq!(
        repo.get_all_sessions().await.len(),
        1,
        "the session must survive a forged access-token revoke"
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
    let config = AppConfig {
        audit: AuditConfig {
            emit_threshold: "debug".to_string(),
            ..Default::default()
        },
        ..make_config()
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
        match outcome {
            AuditOutcome::Failure { reason } => {
                assert!(!reason.is_empty());
                // The fixed reason must not echo the offending sid value.
                assert!(
                    !reason.contains(LEGACY_HASH_SID),
                    "rejection reason must not echo the sid value: {reason}"
                );
            }
            other => panic!("expected Failure outcomes, got {other:?}"),
        }
    }
    let reasons: Vec<_> = rejections
        .iter()
        .map(|(_, o)| match o {
            AuditOutcome::Failure { reason } => reason.clone(),
            AuditOutcome::Success => String::new(),
        })
        .collect();
    for pair in reasons.windows(2) {
        assert_eq!(
            pair[0], pair[1],
            "every unusable-sid rejection carries the same fixed reason"
        );
    }
}

/// A forged signature stays entirely silent even under a debug audit
/// threshold — verification failure precedes any claim inspection.
#[tokio::test]
async fn forged_signature_stays_silent_under_debug_threshold() {
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
        0,
        "verification failure emits nothing at all, got {:?}",
        events
            .iter()
            .map(|e| e.event_type.clone())
            .collect::<Vec<_>>()
    );
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
                .expect("exchange issues a token"),
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
