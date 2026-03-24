use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use http_body_util::BodyExt;
use serde_json::json;
use tower::ServiceExt;

use oidc_exchange::routes::{internal_routes, public_routes};
use oidc_exchange::state::AppState;
use oidc_exchange_core::config::AppConfig;
use oidc_exchange_core::ports::IdentityProvider;
use oidc_exchange_core::service::AppService;
use oidc_exchange_test_utils::{
    MockAuditLog, MockIdentityProvider, MockKeyManager, MockRepository, MockUserSync,
};

const TEST_SECRET: &str = "test-internal-secret-e2e";

fn build_e2e_app() -> Router {
    let provider = MockIdentityProvider::new("test");
    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    providers.insert("test".to_string(), Box::new(provider));

    let mut config = AppConfig::default();
    config.server.issuer = "https://auth.example.com".to_string();
    config.internal_api.enabled = true;
    config.internal_api.shared_secret = Some(TEST_SECRET.to_string());

    let service = AppService::new(
        Box::new(MockRepository::new()),
        Box::new(MockKeyManager::new()),
        Box::new(MockAuditLog::new()),
        Box::new(MockUserSync::new()),
        providers,
        config.clone(),
    );

    let state = AppState {
        service: Arc::new(service),
        config: Arc::new(config),
    };

    public_routes()
        .merge(internal_routes(state.clone()))
        .with_state(state)
}

fn build_e2e_app_with_config(config: AppConfig) -> Router {
    let provider = MockIdentityProvider::new("test");
    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    providers.insert("test".to_string(), Box::new(provider));

    let service = AppService::new(
        Box::new(MockRepository::new()),
        Box::new(MockKeyManager::new()),
        Box::new(MockAuditLog::new()),
        Box::new(MockUserSync::new()),
        providers,
        config.clone(),
    );

    let state = AppState {
        service: Arc::new(service),
        config: Arc::new(config),
    };

    public_routes()
        .merge(internal_routes(state.clone()))
        .with_state(state)
}

async fn body_to_json(body: Body) -> serde_json::Value {
    let bytes = body.collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

/// Decode the payload (second segment) of a JWT without verifying the signature.
fn decode_jwt_payload(jwt: &str) -> serde_json::Value {
    let parts: Vec<&str> = jwt.split('.').collect();
    assert_eq!(parts.len(), 3, "JWT must have 3 segments");
    let payload_bytes = URL_SAFE_NO_PAD.decode(parts[1]).expect("valid base64url");
    serde_json::from_slice(&payload_bytes).expect("valid JSON payload")
}

// ===========================================================================
// Test 1: Full auth flow — exchange → refresh → revoke → refresh fails
// ===========================================================================

#[tokio::test]
async fn e2e_full_auth_flow() {
    let app = build_e2e_app();

    // Step 1: POST /token with grant_type=authorization_code → get access_token + refresh_token
    let exchange_body =
        "grant_type=authorization_code&code=test-code&redirect_uri=http://localhost/callback&provider=test";

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/token")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(exchange_body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let exchange_json = body_to_json(response.into_body()).await;
    let access_token = exchange_json["access_token"].as_str().unwrap();
    let refresh_token = exchange_json["refresh_token"].as_str().unwrap();
    assert!(!access_token.is_empty());
    assert!(!refresh_token.is_empty());
    assert_eq!(exchange_json["token_type"], "Bearer");

    // Step 2: POST /token with grant_type=refresh_token → get new access_token
    let refresh_body = format!(
        "grant_type=refresh_token&refresh_token={}",
        refresh_token
    );

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/token")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(refresh_body.clone()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let refresh_json = body_to_json(response.into_body()).await;
    let new_access_token = refresh_json["access_token"].as_str().unwrap();
    assert!(!new_access_token.is_empty());
    assert_eq!(refresh_json["token_type"], "Bearer");

    // Step 3: POST /revoke with the refresh token → 200
    let revoke_body = format!(
        "token={}&token_type_hint=refresh_token",
        refresh_token
    );

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/revoke")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(revoke_body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    // Step 4: POST /token with grant_type=refresh_token (same token) → should fail
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/token")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(refresh_body))
                .unwrap(),
        )
        .await
        .unwrap();

    // The session was revoked, so refresh should fail
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let error_json = body_to_json(response.into_body()).await;
    assert_eq!(error_json["error"], "invalid_token");
}

// ===========================================================================
// Test 2: Internal API + custom claims in JWT
// ===========================================================================

#[tokio::test]
async fn e2e_internal_api_custom_claims() {
    let app = build_e2e_app();

    // Step 1: POST /internal/users → create user
    // Use external_id "test-subject" to match the mock provider's identity claims
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/internal/users")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", TEST_SECRET))
                .body(Body::from(
                    json!({
                        "external_id": "test-subject",
                        "provider": "test",
                        "email": "test@example.com",
                        "display_name": "Test User"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);

    let user_json = body_to_json(response.into_body()).await;
    let user_id = user_json["id"].as_str().unwrap().to_string();
    assert!(user_id.starts_with("usr_"));

    // Step 2: PUT /internal/users/{id}/claims → set claims {"role": "admin"}
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/internal/users/{}/claims", user_id))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", TEST_SECRET))
                .body(Body::from(json!({"role": "admin"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    // Step 3: POST /token with grant_type=authorization_code (for that user) → get access_token
    // The mock provider returns external_id="test-subject", matching the user we created
    let exchange_body =
        "grant_type=authorization_code&code=test-code&redirect_uri=http://localhost/callback&provider=test";

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/token")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(exchange_body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let token_json = body_to_json(response.into_body()).await;
    let access_token = token_json["access_token"].as_str().unwrap();

    // Step 4: Decode the access_token JWT, verify "role": "admin" is in the claims
    let payload = decode_jwt_payload(access_token);
    assert_eq!(payload["sub"], user_id);
    assert_eq!(payload["iss"], "https://auth.example.com");
    assert_eq!(payload["role"], "admin", "custom claim 'role' should be 'admin' in the JWT");
}

// ===========================================================================
// Test 3: Registration policy — existing_users_only mode
// ===========================================================================

#[tokio::test]
async fn e2e_registration_policy_existing_users_only() {
    let mut config = AppConfig::default();
    config.server.issuer = "https://auth.example.com".to_string();
    config.registration.mode = "existing_users_only".to_string();
    config.internal_api.enabled = true;
    config.internal_api.shared_secret = Some(TEST_SECRET.to_string());

    let app = build_e2e_app_with_config(config);

    // Step 1: POST /token → 403 access_denied (user doesn't exist)
    let exchange_body =
        "grant_type=authorization_code&code=test-code&redirect_uri=http://localhost/callback&provider=test";

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/token")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(exchange_body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let error_json = body_to_json(response.into_body()).await;
    assert_eq!(error_json["error"], "access_denied");

    // Step 2: POST /internal/users → create user with matching external_id
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/internal/users")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", TEST_SECRET))
                .body(Body::from(
                    json!({
                        "external_id": "test-subject",
                        "provider": "test",
                        "email": "test@example.com"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);

    // Step 3: POST /token → 200 success (user now exists)
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/token")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(exchange_body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let token_json = body_to_json(response.into_body()).await;
    assert!(token_json.get("access_token").is_some());
    assert!(token_json.get("refresh_token").is_some());
    assert_eq!(token_json["token_type"], "Bearer");
}
