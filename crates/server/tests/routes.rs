use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::middleware::from_fn;
use axum::Router;
use http_body_util::BodyExt;
use tower::ServiceExt;

use oidc_exchange::middleware::audit_context::audit_context_layer;
use oidc_exchange::routes::public_routes;
use oidc_exchange::state::AppState;
use oidc_exchange_core::config::AppConfig;
use oidc_exchange_core::ports::IdentityProvider;
use oidc_exchange_core::service::AppService;
use oidc_exchange_test_utils::{
    MockAuditLog, MockIdentityProvider, MockKeyManager, MockRepository, MockUserSync,
};

/// Build a router over the public routes with the same `audit_context`
/// middleware `bootstrap::build_router` installs in production, so handler
/// tests exercise `Extension<AuditContext>` the way a real request would.
/// Returns the session-store handle alongside the router so tests can
/// inspect what got persisted after a request.
fn build_test_app() -> (Router, MockRepository) {
    let provider = MockIdentityProvider::new("test");
    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    providers.insert("test".to_string(), Box::new(provider));

    let mut config = AppConfig::default();
    config.server.issuer = "https://auth.example.com".to_string();

    let session_repo = MockRepository::new();

    let service = AppService::new(
        Box::new(MockRepository::new()),
        Box::new(session_repo.clone()),
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
        .layer(from_fn(audit_context_layer))
        .with_state(state);

    (app, session_repo)
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
    let (app, _session_repo) = build_test_app();

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
    let (app, _session_repo) = build_test_app();

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
    let (app, _session_repo) = build_test_app();

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
    assert!(json["error_description"].as_str().unwrap().contains("code"));
}

// ---------------------------------------------------------------------------
// 4. POST /revoke returns 200
// ---------------------------------------------------------------------------

#[tokio::test]
async fn revoke_returns_200() {
    let (app, _session_repo) = build_test_app();

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
// 4a. POST /revoke returns 503 (not a false 200) when the session store fails
// ---------------------------------------------------------------------------

#[tokio::test]
async fn revoke_store_failure_returns_503_refresh_token() {
    let (app, session_repo) = build_test_app();

    // Establish a real session via /token exchange, then make the store
    // unreachable — the mock's fail mode only fires on the revoke calls, so
    // the session genuinely exists (proving the 503 is a store failure, not
    // an idempotent-delete-of-nothing).
    let exchange_body =
        "grant_type=authorization_code&code=test-code&redirect_uri=http://localhost/callback&provider=test";
    let exchange_response = app
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
    assert_eq!(exchange_response.status(), StatusCode::OK);
    let exchange_json = body_to_json(exchange_response.into_body()).await;
    let refresh_token = exchange_json["refresh_token"].as_str().unwrap().to_string();

    session_repo.set_session_fail_mode(true).await;

    let revoke_body = format!("token={refresh_token}&token_type_hint=refresh_token");
    let response = app
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

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["error"], "server_error");
    // Negative-space: the client-facing body must be generic — no store
    // internals ("mock session store failure") leak into the response.
    assert_eq!(json["error_description"], "internal server error");
    assert!(!json["error_description"]
        .as_str()
        .unwrap()
        .contains("mock session store failure"));
}

// ---------------------------------------------------------------------------
// 4a-i. POST /revoke returns 503 (not a false 200) when the presence-check
// LOOKUP itself fails — distinct from the mutating `revoke_session` call
// failing above. A store outage on the read must not collapse to "unknown
// token" and a false 200.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn revoke_lookup_failure_returns_503_refresh_token() {
    let (app, session_repo) = build_test_app();

    // Only the presence-check read fails; no session needs to exist for
    // this to matter — an infrastructure error on the lookup must never be
    // swallowed into `None` (unknown token).
    session_repo.set_session_lookup_fail_mode(true).await;

    let revoke_body = "token=some-refresh-token&token_type_hint=refresh_token";
    let response = app
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

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["error"], "server_error");
    // Negative-space: the client-facing body must be generic — no store
    // internals ("mock session store lookup failure") leak into the response.
    assert_eq!(json["error_description"], "internal server error");
    assert!(!json["error_description"]
        .as_str()
        .unwrap()
        .contains("mock session store lookup failure"));
}

// ---------------------------------------------------------------------------
// 4a-ii. POST /revoke for a genuinely-unknown refresh token (the store's
// presence-check lookup succeeds with `Ok(None)`) still returns 200 — the
// lookup-failure propagation above must not affect the ordinary
// unknown-token carve-out required by RFC 7009.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn revoke_unknown_refresh_token_lookup_ok_none_returns_200() {
    let (app, session_repo) = build_test_app();

    // No fail mode is set, so the lookup succeeds with `Ok(None)` for a
    // token that was never issued.
    session_repo.set_session_lookup_fail_mode(false).await;

    let revoke_body = "token=never-issued-refresh-token&token_type_hint=refresh_token";
    let response = app
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
}

#[tokio::test]
async fn revoke_store_failure_returns_503_access_token() {
    let (app, session_repo) = build_test_app();

    let exchange_body =
        "grant_type=authorization_code&code=test-code&redirect_uri=http://localhost/callback&provider=test";
    let exchange_response = app
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
    assert_eq!(exchange_response.status(), StatusCode::OK);
    let exchange_json = body_to_json(exchange_response.into_body()).await;
    let access_token = exchange_json["access_token"].as_str().unwrap().to_string();

    session_repo.set_session_fail_mode(true).await;

    let revoke_body = format!("token={access_token}&token_type_hint=access_token");
    let response = app
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

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["error"], "server_error");
    assert_eq!(json["error_description"], "internal server error");
}

// ---------------------------------------------------------------------------
// 4b. POST /revoke swallows a token-verification failure — still 200, and
//     never reaches the session store (best-effort carve-out is preserved)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn revoke_access_token_verification_failure_returns_200_no_propagation() {
    let (app, session_repo) = build_test_app();

    // Even with the store unreachable, a malformed/unsigned access token
    // must never reach `revoke_all_user_sessions` — verification fails
    // first, so the store failure is never observed and 200 still comes
    // back per RFC 7009 (invalid/unknown token state is never leaked).
    session_repo.set_session_fail_mode(true).await;

    let revoke_body = "token=not.a-valid.jwt&token_type_hint=access_token";
    let response = app
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
}

// ---------------------------------------------------------------------------
// 4c. POST /revoke with an empty token is rejected as invalid_request
// ---------------------------------------------------------------------------

#[tokio::test]
async fn revoke_empty_token_returns_400() {
    let (app, _session_repo) = build_test_app();

    let body = "token=&token_type_hint=refresh_token";

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

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["error"], "invalid_request");
}

// ---------------------------------------------------------------------------
// 5. GET /keys returns JWKS
// ---------------------------------------------------------------------------

#[tokio::test]
async fn keys_returns_jwks() {
    let (app, _session_repo) = build_test_app();

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
    let (app, _session_repo) = build_test_app();

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
    let (app, _session_repo) = build_test_app();

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

// ---------------------------------------------------------------------------
// 8. POST /token with audit headers stores their values on the session
// ---------------------------------------------------------------------------

#[tokio::test]
async fn token_exchange_with_audit_headers_stores_session_context() {
    let (app, session_repo) = build_test_app();

    let body = "grant_type=authorization_code&code=test-code&redirect_uri=http://localhost/callback&provider=test";

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/token")
                .header("content-type", "application/x-www-form-urlencoded")
                .header("x-forwarded-for", "203.0.113.7")
                .header("user-agent", "audit-test-client/1.0")
                .header("x-device-id", "device-42")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let sessions = session_repo.get_all_sessions().await;
    assert_eq!(sessions.len(), 1, "expected exactly one stored session");
    let session = &sessions[0];
    assert_eq!(session.ip_address.as_deref(), Some("203.0.113.7"));
    assert_eq!(session.user_agent.as_deref(), Some("audit-test-client/1.0"));
    assert_eq!(session.device_id.as_deref(), Some("device-42"));
}

// ---------------------------------------------------------------------------
// 9. POST /token without audit headers stores None client context
// (negative space: the handler must pass through the middleware's `None`
// defaults, not synthesize empty strings).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn token_exchange_without_audit_headers_stores_none_session_context() {
    let (app, session_repo) = build_test_app();

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

    let sessions = session_repo.get_all_sessions().await;
    assert_eq!(sessions.len(), 1, "expected exactly one stored session");
    let session = &sessions[0];
    assert_eq!(session.ip_address, None);
    assert_eq!(session.user_agent, None);
    assert_eq!(session.device_id, None);
}
