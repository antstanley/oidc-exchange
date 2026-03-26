use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
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

const TEST_SECRET: &str = "test-internal-secret-1234";

fn build_test_app() -> Router {
    let provider = MockIdentityProvider::new("test");
    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    providers.insert("test".to_string(), Box::new(provider));

    let mut config = AppConfig::default();
    config.server.issuer = "https://auth.example.com".to_string();
    config.internal_api.enabled = true;
    config.internal_api.shared_secret = Some(TEST_SECRET.to_string());

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

    public_routes()
        .merge(internal_routes(state.clone()))
        .with_state(state)
}

async fn body_to_json(body: Body) -> serde_json::Value {
    let bytes = body.collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

// ---------------------------------------------------------------------------
// 1. Internal auth rejection: no auth header → 401
// ---------------------------------------------------------------------------

#[tokio::test]
async fn internal_auth_rejects_missing_auth() {
    let app = build_test_app();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/internal/users")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"external_id": "ext1", "provider": "google"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["error"], "unauthorized");
}

// ---------------------------------------------------------------------------
// 2. Internal auth rejection: wrong secret → 401
// ---------------------------------------------------------------------------

#[tokio::test]
async fn internal_auth_rejects_wrong_secret() {
    let app = build_test_app();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/internal/users")
                .header("content-type", "application/json")
                .header("authorization", "Bearer wrong-secret")
                .body(Body::from(
                    json!({"external_id": "ext1", "provider": "google"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["error"], "unauthorized");
}

// ---------------------------------------------------------------------------
// 3. Internal auth with correct secret → proceeds to handler
// ---------------------------------------------------------------------------

#[tokio::test]
async fn internal_auth_passes_with_correct_secret() {
    let app = build_test_app();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/internal/users")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", TEST_SECRET))
                .body(Body::from(
                    json!({"external_id": "ext1", "provider": "google", "email": "user@example.com"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    // Should not be 401 — handler should have processed the request
    assert_ne!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(response.status(), StatusCode::CREATED);
}

// ---------------------------------------------------------------------------
// 4. Create user → 201 with user JSON
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_user_returns_201() {
    let app = build_test_app();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/internal/users")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", TEST_SECRET))
                .body(Body::from(
                    json!({
                        "external_id": "ext-123",
                        "provider": "google",
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

    let json = body_to_json(response.into_body()).await;
    assert!(json["id"].as_str().unwrap().starts_with("usr_"));
    assert_eq!(json["external_id"], "ext-123");
    assert_eq!(json["provider"], "google");
    assert_eq!(json["email"], "test@example.com");
    assert_eq!(json["display_name"], "Test User");
    assert_eq!(json["status"], "active");
}

// ---------------------------------------------------------------------------
// 5. Get user → 200 with user JSON
// ---------------------------------------------------------------------------

#[tokio::test]
async fn get_user_returns_200() {
    let app = build_test_app();

    // First create a user
    let create_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/internal/users")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", TEST_SECRET))
                .body(Body::from(
                    json!({
                        "external_id": "ext-get",
                        "provider": "google",
                        "email": "get@example.com"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(create_resp.status(), StatusCode::CREATED);
    let created = body_to_json(create_resp.into_body()).await;
    let user_id = created["id"].as_str().unwrap();

    // Now get the user
    let get_resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/internal/users/{}", user_id))
                .header("authorization", format!("Bearer {}", TEST_SECRET))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(get_resp.status(), StatusCode::OK);

    let json = body_to_json(get_resp.into_body()).await;
    assert_eq!(json["id"], user_id);
    assert_eq!(json["external_id"], "ext-get");
    assert_eq!(json["email"], "get@example.com");
}

// ---------------------------------------------------------------------------
// 6. Claims PATCH merge: create user, PATCH claims, GET → merged
// ---------------------------------------------------------------------------

#[tokio::test]
async fn claims_merge_works() {
    let app = build_test_app();

    // Create user
    let create_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/internal/users")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", TEST_SECRET))
                .body(Body::from(
                    json!({
                        "external_id": "ext-claims",
                        "provider": "google"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    let created = body_to_json(create_resp.into_body()).await;
    let user_id = created["id"].as_str().unwrap();

    // PUT initial claims {"a": 1}
    let put_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/internal/users/{}/claims", user_id))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", TEST_SECRET))
                .body(Body::from(json!({"a": 1}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(put_resp.status(), StatusCode::OK);

    // PATCH merge claims {"b": 2}
    let patch_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!("/internal/users/{}/claims", user_id))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", TEST_SECRET))
                .body(Body::from(json!({"b": 2}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(patch_resp.status(), StatusCode::OK);

    // GET claims → should have both "a" and "b"
    let get_resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/internal/users/{}/claims", user_id))
                .header("authorization", format!("Bearer {}", TEST_SECRET))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(get_resp.status(), StatusCode::OK);

    let claims = body_to_json(get_resp.into_body()).await;
    assert_eq!(claims["a"], 1);
    assert_eq!(claims["b"], 2);
}

// ---------------------------------------------------------------------------
// 7. Delete user → 200
// ---------------------------------------------------------------------------

#[tokio::test]
async fn delete_user_returns_200() {
    let app = build_test_app();

    // Create user first
    let create_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/internal/users")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", TEST_SECRET))
                .body(Body::from(
                    json!({
                        "external_id": "ext-delete",
                        "provider": "google"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    let created = body_to_json(create_resp.into_body()).await;
    let user_id = created["id"].as_str().unwrap();

    // Delete user
    let del_resp = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/internal/users/{}", user_id))
                .header("authorization", format!("Bearer {}", TEST_SECRET))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(del_resp.status(), StatusCode::OK);
}
