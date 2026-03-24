use std::collections::HashMap;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use sha2::{Digest, Sha256};

use oidc_exchange_core::config::{AppConfig, ServerConfig, TokenConfig};
use oidc_exchange_core::domain::{AccessTokenClaims, UserPatch, UserStatus};
use oidc_exchange_core::error::Error;
use oidc_exchange_core::ports::{IdentityProvider, Repository};
use oidc_exchange_core::service::exchange::ExchangeRequest;
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
        Box::new(repo),
        Box::new(MockKeyManager::new()),
        Box::new(MockAuditLog::new()),
        Box::new(MockUserSync::new()),
        providers,
        make_config(),
    )
}

#[tokio::test]
async fn exchange_happy_path_creates_user_and_returns_tokens() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let svc = make_service(repo.clone(), provider);

    let request = ExchangeRequest {
        code: "auth-code-123".to_string(),
        redirect_uri: "https://app.test.com/callback".to_string(),
        provider: "mock".to_string(),
    };

    let response = svc.exchange(request).await.expect("exchange should succeed");

    // Should return a Bearer token response
    assert_eq!(response.token_type, "Bearer");
    assert_eq!(response.expires_in, 900); // 15m = 900s
    assert!(!response.access_token.is_empty());
    assert!(response.refresh_token.is_some());

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

    // A new user should have been created
    let users = repo.get_all_users().await;
    assert_eq!(users.len(), 1);
    assert_eq!(users[0].external_id, "test-subject");
    assert_eq!(users[0].email.as_deref(), Some("test@example.com"));
    assert_eq!(users[0].provider, "mock");

    // A session should have been stored with the hashed refresh token
    let sessions = repo.get_all_sessions().await;
    assert_eq!(sessions.len(), 1);

    let refresh_token = response.refresh_token.unwrap();
    let expected_hash = hex::encode(Sha256::digest(refresh_token.as_bytes()));
    assert_eq!(sessions[0].refresh_token_hash, expected_hash);
    assert_eq!(sessions[0].user_id, users[0].id);
    assert_eq!(sessions[0].provider, "mock");
}

#[tokio::test]
async fn exchange_existing_user_does_not_create_new() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let svc = make_service(repo.clone(), provider);

    // First exchange: creates user
    let request1 = ExchangeRequest {
        code: "code-1".to_string(),
        redirect_uri: "https://app.test.com/callback".to_string(),
        provider: "mock".to_string(),
    };
    let resp1 = svc
        .exchange(request1)
        .await
        .expect("first exchange should succeed");

    // Second exchange: same external_id, should reuse user
    let request2 = ExchangeRequest {
        code: "code-2".to_string(),
        redirect_uri: "https://app.test.com/callback".to_string(),
        provider: "mock".to_string(),
    };
    let resp2 = svc
        .exchange(request2)
        .await
        .expect("second exchange should succeed");

    // Still only one user
    let users = repo.get_all_users().await;
    assert_eq!(users.len(), 1);

    // But two sessions
    let sessions = repo.get_all_sessions().await;
    assert_eq!(sessions.len(), 2);

    // Both tokens should reference the same user
    let payload1 = URL_SAFE_NO_PAD
        .decode(resp1.access_token.split('.').nth(1).unwrap())
        .unwrap();
    let claims1: AccessTokenClaims = serde_json::from_slice(&payload1).unwrap();

    let payload2 = URL_SAFE_NO_PAD
        .decode(resp2.access_token.split('.').nth(1).unwrap())
        .unwrap();
    let claims2: AccessTokenClaims = serde_json::from_slice(&payload2).unwrap();

    assert_eq!(claims1.sub, claims2.sub);
}

#[tokio::test]
async fn exchange_suspended_user_is_rejected() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let svc = make_service(repo.clone(), provider);

    // First exchange creates the user
    let request = ExchangeRequest {
        code: "code".to_string(),
        redirect_uri: "https://app.test.com/callback".to_string(),
        provider: "mock".to_string(),
    };
    svc.exchange(request)
        .await
        .expect("first exchange should succeed");

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

    // Second exchange should fail
    let request2 = ExchangeRequest {
        code: "code-2".to_string(),
        redirect_uri: "https://app.test.com/callback".to_string(),
        provider: "mock".to_string(),
    };
    let err = svc
        .exchange(request2)
        .await
        .expect_err("exchange should fail for suspended user");

    match err {
        Error::UserSuspended { user_id: id } => {
            assert_eq!(id, user_id);
        }
        other => panic!("expected UserSuspended, got: {:?}", other),
    }
}

#[tokio::test]
async fn exchange_unknown_provider_is_rejected() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let svc = make_service(repo, provider);

    let request = ExchangeRequest {
        code: "code".to_string(),
        redirect_uri: "https://app.test.com/callback".to_string(),
        provider: "nonexistent".to_string(),
    };
    let err = svc
        .exchange(request)
        .await
        .expect_err("exchange should fail for unknown provider");

    match err {
        Error::UnknownProvider { provider } => {
            assert_eq!(provider, "nonexistent");
        }
        other => panic!("expected UnknownProvider, got: {:?}", other),
    }
}
