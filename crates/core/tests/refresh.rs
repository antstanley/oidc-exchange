use std::collections::HashMap;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{Duration, Utc};
use sha2::{Digest, Sha256};

use oidc_exchange_core::config::{AppConfig, ServerConfig, TokenConfig};
use oidc_exchange_core::domain::{AccessTokenClaims, Session, UserPatch, UserStatus};
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

/// Helper: perform an exchange to get a refresh token, then return it along
/// with the service and repo for further testing.
async fn exchange_and_get_refresh_token(_repo: &MockRepository, svc: &AppService) -> String {
    let request = ExchangeRequest {
        code: Some("auth-code-123".to_string()),
        redirect_uri: Some("https://app.test.com/callback".to_string()),
        id_token: None,
        provider: "mock".to_string(),
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
    let request = RefreshRequest { refresh_token };
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
    let request = RefreshRequest { refresh_token };
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
