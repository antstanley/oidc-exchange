use std::collections::HashMap;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use sha2::{Digest, Sha256};

use oidc_exchange_core::config::{AppConfig, ServerConfig, TokenConfig};
use oidc_exchange_core::domain::{AccessTokenClaims, AuditEventType, AuditOutcome};
use oidc_exchange_core::ports::{IdentityProvider, SessionRepository};
use oidc_exchange_core::service::exchange::ExchangeRequest;
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
        Box::new(oidc_exchange_test_utils::MockRateLimiter::new()),
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
        Box::new(oidc_exchange_test_utils::MockRateLimiter::new()),
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
async fn revoke_access_token_removes_all_user_sessions() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let svc = make_service(repo.clone(), provider);

    // Exchange twice to create two sessions for the same user
    let response1 = do_exchange(&svc).await;
    let _response2 = do_exchange(&svc).await;

    // Verify two sessions exist
    let sessions = repo.get_all_sessions().await;
    assert_eq!(sessions.len(), 2, "should have two sessions");

    // Verify both sessions belong to the same user
    let user_id = sessions[0].user_id.clone();
    assert_eq!(
        sessions[1].user_id, user_id,
        "both sessions should belong to same user"
    );

    // Revoke using the access token from the first exchange
    let revoke_req = RevokeRequest {
        token: response1.access_token.clone(),
        token_type_hint: Some("access_token".to_string()),
        ..Default::default()
    };
    svc.revoke(revoke_req).await.expect("revoke should succeed");

    // Verify ALL sessions for that user are removed
    let sessions = repo.get_all_sessions().await;
    assert_eq!(
        sessions.len(),
        0,
        "all sessions should be removed after access token revocation"
    );

    // Verify the access token's sub claim matched the user
    let parts: Vec<&str> = response1.access_token.split('.').collect();
    let payload_bytes = URL_SAFE_NO_PAD
        .decode(parts[1])
        .expect("payload should be valid base64url");
    let claims: AccessTokenClaims =
        serde_json::from_slice(&payload_bytes).expect("payload should deserialize");
    assert_eq!(claims.sub, user_id);
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
async fn revoke_valid_access_token_emits_all_sessions_revoked() {
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
        "a verified access-token revoke must emit exactly one audit event"
    );
    let revoke_event = events.last().expect("an event was just recorded");
    assert_eq!(revoke_event.event_type, AuditEventType::AllSessionsRevoked);
    assert_eq!(revoke_event.outcome, AuditOutcome::Success);
    assert_eq!(revoke_event.actor, Some(user_id));
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
