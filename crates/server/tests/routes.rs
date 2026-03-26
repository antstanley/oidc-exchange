use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use tower::ServiceExt;

use oidc_exchange::routes::public_routes;
use oidc_exchange::state::AppState;
use oidc_exchange_core::config::AppConfig;
use oidc_exchange_core::ports::IdentityProvider;
use oidc_exchange_core::service::AppService;
use oidc_exchange_test_utils::{
    MockAuditLog, MockIdentityProvider, MockKeyManager, MockRepository, MockUserSync,
};

fn build_test_app() -> Router {
    let provider = MockIdentityProvider::new("test");
    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    providers.insert("test".to_string(), Box::new(provider));

    let mut config = AppConfig::default();
    config.server.issuer = "https://auth.example.com".to_string();

    let service = AppService::new(
        Box::new(MockRepository::new()),
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

    public_routes().with_state(state)
}

async fn body_to_json(body: Body) -> serde_json::Value {
    let bytes = body.collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

// ---------------------------------------------------------------------------
// 1. POST /token exchange returns 200 with access_token
// ---------------------------------------------------------------------------

#[tokio::test]
async fn token_exchange_returns_200_with_access_token() {
    let app = build_test_app();

    let body = "grant_type=authorization_code&code=test-code&redirect_uri=http://localhost/callback&provider=test";

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/token")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = body_to_json(response.into_body()).await;
    assert!(json.get("access_token").is_some());
    assert!(json.get("refresh_token").is_some());
    assert_eq!(json["token_type"], "Bearer");
    assert!(json.get("expires_in").is_some());
}

// ---------------------------------------------------------------------------
// 2. POST /token with invalid grant_type returns 400
// ---------------------------------------------------------------------------

#[tokio::test]
async fn token_invalid_grant_type_returns_400() {
    let app = build_test_app();

    let body = "grant_type=client_credentials";

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/token")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["error"], "unsupported_grant_type");
}

// ---------------------------------------------------------------------------
// 3. POST /token with missing code returns 400
// ---------------------------------------------------------------------------

#[tokio::test]
async fn token_missing_code_returns_400() {
    let app = build_test_app();

    let body = "grant_type=authorization_code&redirect_uri=http://localhost/callback&provider=test";

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/token")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["error"], "invalid_request");
    assert!(json["error_description"]
        .as_str()
        .unwrap()
        .contains("code"));
}

// ---------------------------------------------------------------------------
// 4. POST /revoke returns 200
// ---------------------------------------------------------------------------

#[tokio::test]
async fn revoke_returns_200() {
    let app = build_test_app();

    let body = "token=some-refresh-token&token_type_hint=refresh_token";

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/revoke")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

// ---------------------------------------------------------------------------
// 5. GET /keys returns JWKS
// ---------------------------------------------------------------------------

#[tokio::test]
async fn keys_returns_jwks() {
    let app = build_test_app();

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/keys")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = body_to_json(response.into_body()).await;
    let keys = json["keys"].as_array().unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0]["kty"], "OKP");
    assert_eq!(keys[0]["crv"], "Ed25519");
    assert_eq!(keys[0]["kid"], "test-key-1");
}

// ---------------------------------------------------------------------------
// 6. GET /.well-known/openid-configuration returns discovery doc
// ---------------------------------------------------------------------------

#[tokio::test]
async fn well_known_returns_discovery_doc() {
    let app = build_test_app();

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/.well-known/openid-configuration")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["issuer"], "https://auth.example.com");
    assert_eq!(json["jwks_uri"], "https://auth.example.com/keys");
    assert_eq!(json["token_endpoint"], "https://auth.example.com/token");
    assert_eq!(
        json["revocation_endpoint"],
        "https://auth.example.com/revoke"
    );

    let grant_types = json["grant_types_supported"].as_array().unwrap();
    assert!(grant_types.iter().any(|v| v == "authorization_code"));
    assert!(grant_types.iter().any(|v| v == "refresh_token"));

    let algs = json["id_token_signing_alg_values_supported"]
        .as_array()
        .unwrap();
    assert!(algs.iter().any(|v| v == "EdDSA"));
}

// ---------------------------------------------------------------------------
// 7. GET /health returns 200
// ---------------------------------------------------------------------------

#[tokio::test]
async fn health_returns_200() {
    let app = build_test_app();

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["status"], "ok");
}
