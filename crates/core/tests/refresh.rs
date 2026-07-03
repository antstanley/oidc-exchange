use std::collections::HashMap;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{Duration, Utc};
use sha2::{Digest, Sha256};

use oidc_exchange_core::config::{AppConfig, AuditConfig, ServerConfig, TokenConfig};
use oidc_exchange_core::domain::{
    AccessTokenClaims, AuditEventType, AuditOutcome, Session, UserPatch, UserStatus,
};
use oidc_exchange_core::error::Error;
use oidc_exchange_core::ports::{IdentityProvider, SessionRepository, UserRepository};
use oidc_exchange_core::service::exchange::ExchangeRequest;
use oidc_exchange_core::service::refresh::RefreshRequest;
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
/// the test can inspect recorded events after the refresh runs.
fn make_service_with_audit(
    repo: MockRepository,
    provider: MockIdentityProvider,
    config: AppConfig,
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
        config,
    )
}

/// Helper: perform an exchange to get a refresh token, then return it along
/// with the service and repo for further testing.
async fn exchange_and_get_refresh_token(_repo: &MockRepository, svc: &AppService) -> String {
    let request = ExchangeRequest {
        code: Some("auth-code-123".to_string()),
        redirect_uri: Some("https://app.test.com/callback".to_string()),
        id_token: None,
        provider: "mock".to_string(),
        ..Default::default()
    };
    let response = svc
        .exchange(request)
        .await
        .expect("exchange should succeed");
    response.refresh_token.expect("should have a refresh token")
}

#[tokio::test]
async fn refresh_happy_path_returns_new_access_token() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let svc = make_service(repo.clone(), provider);

    // First do an exchange to get a refresh token
    let refresh_token = exchange_and_get_refresh_token(&repo, &svc).await;

    // Now use the refresh token
    let request = RefreshRequest {
        refresh_token: refresh_token.clone(),
        ..Default::default()
    };
    let response = svc.refresh(request).await.expect("refresh should succeed");

    // Verify the response
    assert_eq!(response.token_type, "Bearer");
    assert_eq!(response.expires_in, 900); // 15m = 900s
    assert!(
        response.refresh_token.is_none(),
        "refresh should not return a new refresh token"
    );
    assert!(!response.access_token.is_empty());

    // Access token should be a valid JWT structure (3 dot-separated parts)
    let parts: Vec<&str> = response.access_token.split('.').collect();
    assert_eq!(parts.len(), 3, "JWT should have 3 parts");

    // Decode and verify the payload claims
    let payload_bytes = URL_SAFE_NO_PAD
        .decode(parts[1])
        .expect("payload should be valid base64url");
    let claims: AccessTokenClaims =
        serde_json::from_slice(&payload_bytes).expect("payload should deserialize");
    assert_eq!(claims.iss, "https://auth.test.com");
    assert_eq!(claims.aud, "https://api.test.com");
    assert!(claims.sub.starts_with("usr_"));

    // The sub should match the user created during exchange
    let users = repo.get_all_users().await;
    assert_eq!(users.len(), 1);
    assert_eq!(claims.sub, users[0].id);
}

#[tokio::test]
async fn refresh_expired_token_returns_invalid_token() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let svc = make_service(repo.clone(), provider);

    // Do an exchange to create a user and session
    let refresh_token = exchange_and_get_refresh_token(&repo, &svc).await;

    // Manually expire the session by replacing it with an expired one
    let token_hash = hex::encode(Sha256::digest(refresh_token.as_bytes()));
    let sessions = repo.get_all_sessions().await;
    let original_session = sessions
        .iter()
        .find(|s| s.refresh_token_hash == token_hash)
        .expect("session should exist");

    // Revoke the original and store an expired copy
    repo.revoke_session(&token_hash)
        .await
        .expect("revoke should succeed");

    let expired_session = Session {
        expires_at: Utc::now() - Duration::hours(1),
        ..original_session.clone()
    };
    repo.store_refresh_token(&expired_session)
        .await
        .expect("store should succeed");

    // Now try to refresh with the expired token
    let request = RefreshRequest {
        refresh_token,
        ..Default::default()
    };
    let err = svc
        .refresh(request)
        .await
        .expect_err("refresh should fail for expired token");

    match err {
        Error::InvalidToken { .. } => {} // expected
        other => panic!("expected InvalidToken, got: {:?}", other),
    }
}

#[tokio::test]
async fn refresh_unknown_token_returns_invalid_token() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let svc = make_service(repo, provider);

    // Try to refresh with a token that was never stored
    let request = RefreshRequest {
        refresh_token: "this-token-does-not-exist".to_string(),
        ..Default::default()
    };
    let err = svc
        .refresh(request)
        .await
        .expect_err("refresh should fail for unknown token");

    match err {
        Error::InvalidToken { .. } => {} // expected
        other => panic!("expected InvalidToken, got: {:?}", other),
    }
}

#[tokio::test]
async fn refresh_suspended_user_returns_user_suspended() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let svc = make_service(repo.clone(), provider);

    // Exchange to create user and session
    let refresh_token = exchange_and_get_refresh_token(&repo, &svc).await;

    // Suspend the user
    let users = repo.get_all_users().await;
    let user_id = users[0].id.clone();
    repo.update_user(
        &user_id,
        &UserPatch {
            status: Some(UserStatus::Suspended),
            email: None,
            display_name: None,
            metadata: None,
            claims: None,
        },
    )
    .await
    .expect("update should succeed");

    // Now try to refresh
    let request = RefreshRequest {
        refresh_token,
        ..Default::default()
    };
    let err = svc
        .refresh(request)
        .await
        .expect_err("refresh should fail for suspended user");

    match err {
        Error::UserSuspended { user_id: id } => {
            assert_eq!(id, user_id);
        }
        other => panic!("expected UserSuspended, got: {:?}", other),
    }
}

/// Negative space: an unknown-token refresh under the default `info`
/// `emit_threshold` audits nothing — `ValidationFailed` is emitted at
/// `debug`, which the default threshold drops before any adapter sees it.
#[tokio::test]
async fn refresh_unknown_token_under_default_threshold_emits_nothing() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let audit = MockAuditLog::new();
    let audit_clone = audit.clone();
    let svc = make_service_with_audit(repo, provider, make_config(), audit);

    let request = RefreshRequest {
        refresh_token: "this-token-does-not-exist".to_string(),
        ip_address: Some("203.0.113.20".to_string()),
        user_agent: Some("test-agent/1.0".to_string()),
        device_id: None,
    };
    svc.refresh(request)
        .await
        .expect_err("refresh should fail for unknown token");

    let events = audit_clone.events().await;
    assert!(
        events.is_empty(),
        "default info threshold must suppress debug-severity ValidationFailed, got: {:?}",
        events.iter().map(|e| &e.event_type).collect::<Vec<_>>()
    );
}

/// Lowering `[audit] emit_threshold` to `debug` surfaces the
/// `ValidationFailed` event that the default threshold suppresses, carrying
/// the request's ip/ua and a `Failure` outcome.
#[tokio::test]
async fn refresh_unknown_token_under_debug_threshold_emits_validation_failed() {
    let config = AppConfig {
        audit: AuditConfig {
            emit_threshold: "debug".to_string(),
            ..Default::default()
        },
        ..make_config()
    };

    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let audit = MockAuditLog::new();
    let audit_clone = audit.clone();
    let svc = make_service_with_audit(repo, provider, config, audit);

    let request = RefreshRequest {
        refresh_token: "this-token-does-not-exist".to_string(),
        ip_address: Some("203.0.113.21".to_string()),
        user_agent: Some("test-agent/2.0".to_string()),
        device_id: None,
    };
    svc.refresh(request)
        .await
        .expect_err("refresh should fail for unknown token");

    let events = audit_clone.events().await;
    assert_eq!(
        events.len(),
        1,
        "lowering the threshold to debug must surface exactly one event, got: {:?}",
        events.iter().map(|e| &e.event_type).collect::<Vec<_>>()
    );
    assert_eq!(events[0].event_type, AuditEventType::ValidationFailed);
    match &events[0].outcome {
        AuditOutcome::Failure { .. } => {}
        other => panic!("expected Failure outcome, got: {:?}", other),
    }
    assert_eq!(events[0].ip_address.as_deref(), Some("203.0.113.21"));
    assert_eq!(events[0].user_agent.as_deref(), Some("test-agent/2.0"));
}

/// An expired-token refresh under a lowered `emit_threshold` also audits
/// `ValidationFailed`, this time with the session's user id as actor since
/// the session (and therefore the user id) was already resolved.
#[tokio::test]
async fn refresh_expired_token_under_debug_threshold_emits_validation_failed_with_actor() {
    let config = AppConfig {
        audit: AuditConfig {
            emit_threshold: "debug".to_string(),
            ..Default::default()
        },
        ..make_config()
    };

    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let svc = make_service(repo.clone(), provider);

    let refresh_token = exchange_and_get_refresh_token(&repo, &svc).await;

    let token_hash = hex::encode(Sha256::digest(refresh_token.as_bytes()));
    let sessions = repo.get_all_sessions().await;
    let original_session = sessions
        .iter()
        .find(|s| s.refresh_token_hash == token_hash)
        .expect("session should exist");
    let user_id = original_session.user_id.clone();

    repo.revoke_session(&token_hash)
        .await
        .expect("revoke should succeed");
    let expired_session = Session {
        expires_at: Utc::now() - Duration::hours(1),
        ..original_session.clone()
    };
    repo.store_refresh_token(&expired_session)
        .await
        .expect("store should succeed");

    let provider2 = MockIdentityProvider::new("mock");
    let audit = MockAuditLog::new();
    let audit_clone = audit.clone();
    let svc2 = make_service_with_audit(repo, provider2, config, audit);

    let request = RefreshRequest {
        refresh_token,
        ..Default::default()
    };
    svc2.refresh(request)
        .await
        .expect_err("refresh should fail for expired token");

    let events = audit_clone.events().await;
    assert_eq!(events.len(), 1, "expected exactly one audit event");
    assert_eq!(events[0].event_type, AuditEventType::ValidationFailed);
    assert_eq!(events[0].actor.as_deref(), Some(user_id.as_str()));
}

/// A suspended-user refresh emits `UserSuspended` (default threshold covers
/// it, since it is not gated behind `emit_threshold`), carrying the user id
/// as actor and the request's ip/ua.
#[tokio::test]
async fn refresh_suspended_user_emits_user_suspended_event() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let svc = make_service(repo.clone(), provider);

    let refresh_token = exchange_and_get_refresh_token(&repo, &svc).await;

    let users = repo.get_all_users().await;
    let user_id = users[0].id.clone();
    repo.update_user(
        &user_id,
        &UserPatch {
            status: Some(UserStatus::Suspended),
            email: None,
            display_name: None,
            metadata: None,
            claims: None,
        },
    )
    .await
    .expect("update should succeed");

    let provider2 = MockIdentityProvider::new("mock");
    let audit = MockAuditLog::new();
    let audit_clone = audit.clone();
    let svc2 = make_service_with_audit(repo, provider2, make_config(), audit);

    let request = RefreshRequest {
        refresh_token,
        ip_address: Some("203.0.113.22".to_string()),
        user_agent: Some("test-agent/3.0".to_string()),
        device_id: None,
    };
    svc2.refresh(request)
        .await
        .expect_err("refresh should fail for suspended user");

    let events = audit_clone.events().await;
    assert_eq!(
        events.len(),
        1,
        "suspended-user refresh must emit exactly one event, got: {:?}",
        events.iter().map(|e| &e.event_type).collect::<Vec<_>>()
    );
    assert_eq!(events[0].event_type, AuditEventType::UserSuspended);
    match &events[0].outcome {
        AuditOutcome::Failure { .. } => {}
        other => panic!("expected Failure outcome, got: {:?}", other),
    }
    assert_eq!(events[0].actor.as_deref(), Some(user_id.as_str()));
    assert_eq!(events[0].ip_address.as_deref(), Some("203.0.113.22"));
    assert_eq!(events[0].user_agent.as_deref(), Some("test-agent/3.0"));
}

/// A successful refresh emits exactly one `TokenRefresh` (info, success)
/// event, carrying the user id as actor and the request's ip/ua — visible
/// under the default `info` `emit_threshold` since `TokenRefresh` is `Info`
/// severity.
#[tokio::test]
async fn refresh_success_emits_token_refresh_event() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let svc = make_service(repo.clone(), provider);

    let refresh_token = exchange_and_get_refresh_token(&repo, &svc).await;

    let users = repo.get_all_users().await;
    let user_id = users[0].id.clone();

    let provider2 = MockIdentityProvider::new("mock");
    let audit = MockAuditLog::new();
    let audit_clone = audit.clone();
    let svc2 = make_service_with_audit(repo, provider2, make_config(), audit);

    let request = RefreshRequest {
        refresh_token,
        ip_address: Some("203.0.113.23".to_string()),
        user_agent: Some("test-agent/4.0".to_string()),
        device_id: None,
    };
    svc2.refresh(request).await.expect("refresh should succeed");

    let events = audit_clone.events().await;
    assert_eq!(
        events.len(),
        1,
        "successful refresh must emit exactly one event, got: {:?}",
        events.iter().map(|e| &e.event_type).collect::<Vec<_>>()
    );
    assert_eq!(events[0].event_type, AuditEventType::TokenRefresh);
    assert_eq!(events[0].outcome, AuditOutcome::Success);
    assert_eq!(events[0].actor.as_deref(), Some(user_id.as_str()));
    assert_eq!(events[0].ip_address.as_deref(), Some("203.0.113.23"));
    assert_eq!(events[0].user_agent.as_deref(), Some("test-agent/4.0"));
}
