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
use oidc_exchange_core::config::{Config, RawConfig};
use oidc_exchange_core::ports::IdentityProvider;
use oidc_exchange_core::service::AppService;
use oidc_exchange_test_utils::{
    MockAuditLog, MockIdentityProvider, MockKeyManager, MockRepository, MockUserSync,
};

const TEST_SECRET: &str = "test-internal-secret-1234";

fn test_config(shared_secret: &str) -> Config {
    let mut raw_config: RawConfig = toml::from_str(include_str!("../../../config/default.toml"))
        .expect("default test config is valid");
    raw_config.server.issuer = "https://auth.example.com".to_string();
    raw_config.internal_api.enabled = true;
    raw_config.internal_api.shared_secret = Some(shared_secret.to_string());
    Config::resolve(raw_config).expect("test config should resolve")
}

fn build_test_app() -> Router {
    let provider = MockIdentityProvider::new("test");
    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    providers.insert("test".to_string(), Box::new(provider));

    let config = test_config(TEST_SECRET);

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
// 3b. Internal auth rejection: empty configured secret is never "configured",
// even against an empty Bearer token (defence in depth — `Config::resolve`
// already refuses to start a role that serves the internal API with an empty
// `shared_secret`, so this only guards a config built by hand, e.g. in tests
// or an embedder).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn internal_auth_rejects_empty_configured_secret_even_with_empty_bearer_token() {
    let provider = MockIdentityProvider::new("test");
    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    providers.insert("test".to_string(), Box::new(provider));

    let mut raw_config: RawConfig = toml::from_str(include_str!("../../../config/default.toml"))
        .expect("default test config is valid");
    raw_config.server.issuer = "https://auth.example.com".to_string();
    raw_config.internal_api.enabled = true;
    raw_config.internal_api.shared_secret = Some("valid-test-secret".to_string());
    let mut config = Config::resolve(raw_config).expect("test config should resolve");
    config.internal_api.shared_secret = None;

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

    let app = public_routes()
        .merge(internal_routes(state.clone()))
        .with_state(state);

    // An empty `Bearer ` token must not be accepted just because the
    // configured secret is also empty.
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/internal/stats")
                .header("authorization", "Bearer ")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["error"], "unauthorized");
    assert_eq!(json["error_description"], "internal API not configured");
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

// ---------------------------------------------------------------------------
// 8. Unknown user id on the mutating internal routes → 404 `not_found`,
// never a 500 `server_error` (negative-space: the pre-check must catch the
// typo before the adapter's `StoreError` backstop would fire).
// ---------------------------------------------------------------------------

const UNKNOWN_USER_ID: &str = "usr_does_not_exist";

#[tokio::test]
async fn update_user_unknown_id_returns_404_not_found() {
    let app = build_test_app();

    let response = app
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!("/internal/users/{}", UNKNOWN_USER_ID))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", TEST_SECRET))
                .body(Body::from(json!({"display_name": "New Name"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_ne!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["error"], "not_found");
}

#[tokio::test]
async fn delete_user_unknown_id_returns_404_not_found() {
    let app = build_test_app();

    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/internal/users/{}", UNKNOWN_USER_ID))
                .header("authorization", format!("Bearer {}", TEST_SECRET))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_ne!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["error"], "not_found");
}

#[tokio::test]
async fn get_claims_unknown_id_returns_404_not_found() {
    let app = build_test_app();

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/internal/users/{}/claims", UNKNOWN_USER_ID))
                .header("authorization", format!("Bearer {}", TEST_SECRET))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_ne!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["error"], "not_found");
}

#[tokio::test]
async fn set_claims_unknown_id_returns_404_not_found() {
    let app = build_test_app();

    let response = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/internal/users/{}/claims", UNKNOWN_USER_ID))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", TEST_SECRET))
                .body(Body::from(json!({"a": 1}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_ne!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["error"], "not_found");
}

#[tokio::test]
async fn merge_claims_unknown_id_returns_404_not_found() {
    let app = build_test_app();

    let response = app
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!("/internal/users/{}/claims", UNKNOWN_USER_ID))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", TEST_SECRET))
                .body(Body::from(json!({"b": 2}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_ne!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["error"], "not_found");
}

#[tokio::test]
async fn clear_claims_unknown_id_returns_404_not_found() {
    let app = build_test_app();

    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/internal/users/{}/claims", UNKNOWN_USER_ID))
                .header("authorization", format!("Bearer {}", TEST_SECRET))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_ne!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["error"], "not_found");
}

// ---------------------------------------------------------------------------
// POST /internal/sessions/cleanup — auth, response shape, and sweep count
// (`04-http-api.md` → Internal routes; the scheduler-driven equivalent of the
// bootstrap-spawned session reaper)
// ---------------------------------------------------------------------------

/// Build the test app over a session store the caller keeps a handle to, so a
/// test can seed rows and then observe what the endpoint swept.
fn build_test_app_with_shared_session_store() -> (Router, MockRepository) {
    let provider = MockIdentityProvider::new("test");
    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    providers.insert("test".to_string(), Box::new(provider));

    let config = test_config(TEST_SECRET);

    let sessions = MockRepository::new();
    let service = AppService::new(
        Box::new(MockRepository::new()),
        Box::new(sessions.clone()),
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

    (
        public_routes()
            .merge(internal_routes(state.clone()))
            .with_state(state),
        sessions,
    )
}

#[tokio::test]
async fn cleanup_endpoint_rejects_missing_auth() {
    let (app, _sessions) = build_test_app_with_shared_session_store();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/internal/sessions/cleanup")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "the cleanup lever sits behind internal auth like every /internal route"
    );
    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["error"], "unauthorized");
}

#[tokio::test]
async fn cleanup_endpoint_rejects_wrong_secret() {
    let (app, _sessions) = build_test_app_with_shared_session_store();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/internal/sessions/cleanup")
                .header("authorization", "Bearer not-the-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["error"], "unauthorized");
}

#[tokio::test]
async fn cleanup_endpoint_returns_zero_for_an_empty_store() {
    let (app, _sessions) = build_test_app_with_shared_session_store();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/internal/sessions/cleanup")
                .header("authorization", format!("Bearer {}", TEST_SECRET))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = body_to_json(response.into_body()).await;
    assert_eq!(
        json["deleted"], 0,
        "an empty store reports zero rows deleted — and nothing else"
    );
    assert!(
        json.get("sessions").is_none() && json.get("users").is_none(),
        "the response must carry no store contents, only the count"
    );
}

/// The endpoint sweeps expired sessions *and* expired retirement records in
/// one call and reports their combined count, leaving live state untouched —
/// the same semantics the scheduled reaper gets from the shared port method.
#[tokio::test]
async fn cleanup_endpoint_sweeps_expired_rows_and_reports_the_combined_count() {
    use oidc_exchange_core::ports::SessionRepository;
    use oidc_exchange_test_utils::session_contract as sc;

    let (app, sessions) = build_test_app_with_shared_session_store();

    // Seed: one live generation, one expired session, one expired retirement
    // record (a past-expiry family rotated once, so its record inherits the
    // past family deadline).
    let base = sc::capture_base_instant();
    let future = base + chrono::Duration::hours(2);
    let past = base - chrono::Duration::hours(1);

    let live_family = sc::fixture_family_id("cleanup-endpoint:live");
    let live = sc::generation_session(
        "usr_cleanup",
        &live_family,
        0,
        sc::fixture_hash("cleanup-endpoint:live:gen0"),
        future,
        base,
        None,
    );
    sessions.store_refresh_token(&live).await.unwrap();

    let dead_family = sc::fixture_family_id("cleanup-endpoint:dead");
    let dead = sc::generation_session(
        "usr_cleanup",
        &dead_family,
        0,
        sc::fixture_hash("cleanup-endpoint:dead:gen0"),
        past,
        base,
        None,
    );
    sessions.store_refresh_token(&dead).await.unwrap();

    let rotting_family = sc::fixture_family_id("cleanup-endpoint:rotting");
    let gen0 = sc::generation_session(
        "usr_cleanup",
        &rotting_family,
        0,
        sc::fixture_hash("cleanup-endpoint:rotting:gen0"),
        past,
        base,
        None,
    );
    let gen1 = sc::generation_session(
        "usr_cleanup",
        &rotting_family,
        1,
        sc::fixture_hash("cleanup-endpoint:rotting:gen1"),
        past,
        base,
        Some(base),
    );
    sessions.store_refresh_token(&gen0).await.unwrap();
    assert!(
        sessions
            .rotate_refresh_token(&gen0.refresh_token_hash, &gen1)
            .await
            .unwrap(),
        "fixture rotation wins its CAS"
    );

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/internal/sessions/cleanup")
                .header("authorization", format!("Bearer {}", TEST_SECRET))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = body_to_json(response.into_body()).await;
    assert_eq!(
        json["deleted"], 3,
        "one call deletes the expired session, the expired successor of the rotated \
         past-expiry family, and its expired retirement record"
    );

    // Live state survives, visible through the same service surface an
    // operator would check.
    assert_eq!(
        sessions.get_all_sessions().await.len(),
        1,
        "only the live generation remains after the sweep"
    );
    assert!(
        sessions.get_all_retired_tokens().await.is_empty(),
        "expired retirement records are swept together with expired sessions"
    );

    // A second call is idempotent: zero further deletions.
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/internal/sessions/cleanup")
                .header("authorization", format!("Bearer {}", TEST_SECRET))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_to_json(response.into_body()).await["deleted"], 0);
}
