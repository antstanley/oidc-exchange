use std::collections::HashMap;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use sha2::{Digest, Sha256};

use oidc_exchange_core::config::{AppConfig, ServerConfig, TokenConfig};
use oidc_exchange_core::domain::AccessTokenClaims;
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
    };
    svc.exchange(request).await.expect("exchange should succeed")
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
    };
    svc.revoke(revoke_req).await.expect("revoke should succeed");

    // Verify session is removed
    let sessions = repo.get_all_sessions().await;
    assert_eq!(sessions.len(), 0, "session should be removed after revocation");

    // Also verify by hash lookup
    let token_hash = hex::encode(Sha256::digest(refresh_token.as_bytes()));
    let session = repo
        .get_session_by_refresh_token(&token_hash)
        .await
        .expect("lookup should not error");
    assert!(session.is_none(), "session should not exist after revocation");
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
    assert_eq!(sessions[1].user_id, user_id, "both sessions should belong to same user");

    // Revoke using the access token from the first exchange
    let revoke_req = RevokeRequest {
        token: response1.access_token.clone(),
        token_type_hint: Some("access_token".to_string()),
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
    };
    let result = svc.revoke(revoke_req).await;
    assert!(result.is_ok(), "revoke should always return Ok per RFC 7009");

    // Also try with access_token hint and a bogus JWT
    let revoke_req = RevokeRequest {
        token: "not.a.valid-jwt".to_string(),
        token_type_hint: Some("access_token".to_string()),
    };
    let result = svc.revoke(revoke_req).await;
    assert!(result.is_ok(), "revoke should always return Ok per RFC 7009");

    // Also try with a completely garbage string (not even JWT-shaped)
    let revoke_req = RevokeRequest {
        token: "garbage".to_string(),
        token_type_hint: Some("access_token".to_string()),
    };
    let result = svc.revoke(revoke_req).await;
    assert!(result.is_ok(), "revoke should always return Ok per RFC 7009");
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
