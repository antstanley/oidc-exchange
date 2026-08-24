use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::middleware::from_fn;
use axum::Router;
use http_body_util::BodyExt;
use tower::ServiceExt;

use oidc_exchange::middleware::audit_context::ffi_audit_context_layer;
use oidc_exchange::routes::{nonce_routes, public_routes};
use oidc_exchange::state::AppState;
use oidc_exchange_core::config::{Config, RawConfig};
use oidc_exchange_core::ports::IdentityProvider;
use oidc_exchange_core::service::AppService;
use oidc_exchange_test_utils::{
    MockAuditLog, MockIdentityProvider, MockKeyManager, MockRepository, MockUserSync,
};

/// Build a router over the public routes with the same `audit_context`
/// middleware `bootstrap::build_router` installs in production, so handler
/// tests exercise `Extension<AuditContext>` the way a real request would.
/// Returns the session-store handle alongside the router so tests can
/// inspect what got persisted after a request, and a clone of the mock
/// identity provider as an observation handle (clones share call counters,
/// so tests can prove a rejected request never reached the provider).
fn build_test_app() -> (Router, MockRepository) {
    let (app, session_repo, _provider) = build_test_app_with_provider();
    (app, session_repo)
}

/// Same router as `build_test_app`, plus the provider observation handle.
fn build_test_app_with_provider() -> (Router, MockRepository, MockIdentityProvider) {
    build_test_app_with_provider_impl(false)
}

/// Same as `build_test_app_with_provider`, with the direct ID-token grant
/// enabled (`grants.id_token = true`) so the strict-parse tests that carry an
/// `id_token` field exercise the parser instead of the handler's grants gate.
fn build_test_app_with_provider_and_grants() -> (Router, MockRepository, MockIdentityProvider) {
    build_test_app_with_provider_impl(true)
}

fn build_test_app_with_provider_impl(
    grants_id_token: bool,
) -> (Router, MockRepository, MockIdentityProvider) {
    let provider = MockIdentityProvider::new("test");
    let observer = provider.clone();
    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    providers.insert("test".to_string(), Box::new(provider));

    let mut raw_config: RawConfig = toml::from_str(include_str!("../../../config/default.toml"))
        .expect("default test config is valid");
    raw_config.server.issuer = "https://auth.example.com".to_string();
    raw_config.grants.id_token = grants_id_token;
    let config = Config::resolve(raw_config).expect("test config should resolve");

    let session_repo = MockRepository::new();

    let service = AppService::new(
        Box::new(MockRepository::new()),
        Box::new(session_repo.clone()),
        Box::new(MockKeyManager::new()),
        Box::new(MockAuditLog::new()),
        Box::new(MockUserSync::new()),
        Box::new(oidc_exchange_adapters::noop::NoopRateLimiter::new()),
        providers,
        config.clone(),
    );

    let rate_limiter = Arc::new(oidc_exchange_adapters::noop::NoopRateLimiter::new());
    let state = AppState {
        service: Arc::new(service),
        config: Arc::new(config),
        rate_limiter,
    };

    let mut app = public_routes();
    if grants_id_token {
        app = app.merge(nonce_routes());
    }
    let app = app
        .layer(from_fn(ffi_audit_context_layer))
        .with_state(state);

    (app, session_repo, observer)
}

async fn body_to_json(body: Body) -> serde_json::Value {
    let bytes = body.collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

/// POST a urlencoded form body to `path` on the test router.
async fn post_form(app: &Router, path: &str, body: &str) -> axum::response::Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
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
    // The strict-parse contract names the offending parameter from its closed
    // table (04-http-api.md → Token-endpoint errors); the set is fixed and
    // never echoes caller-supplied values or internal detail.
    assert_eq!(
        json["error_description"],
        "missing required parameter: code"
    );
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
// 4b-ii. POST /revoke with a valid access token removes only the session the
// token's sid names: the revoked session's refresh token stops working while
// its same-user sibling keeps refreshing (end-to-end authority model).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn revoke_valid_access_token_removes_only_its_own_session() {
    let (app, session_repo) = build_test_app();

    // Two exchanges for the same user → two independent sessions.
    let exchange_body =
        "grant_type=authorization_code&code=test-code&redirect_uri=http://localhost/callback&provider=test";
    let first = app
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
    assert_eq!(first.status(), StatusCode::OK);
    let first_json = body_to_json(first.into_body()).await;
    let access_token = first_json["access_token"].as_str().unwrap().to_string();
    let refresh1 = first_json["refresh_token"].as_str().unwrap().to_string();

    let second_body =
        "grant_type=authorization_code&code=other-code&redirect_uri=http://localhost/callback&provider=test";
    let second = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/token")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(second_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::OK);
    let second_json = body_to_json(second.into_body()).await;
    let refresh2 = second_json["refresh_token"].as_str().unwrap().to_string();

    // Two sessions exist before revocation.
    assert_eq!(session_repo.get_all_sessions().await.len(), 2);

    // Revoke with the first exchange's access token.
    let revoke_body = format!("token={access_token}&token_type_hint=access_token");
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

    // Exactly one session survives.
    assert_eq!(
        session_repo.get_all_sessions().await.len(),
        1,
        "only the sid-named session may be removed"
    );

    // The revoked session's refresh token is dead...
    let refresh_body = format!("grant_type=refresh_token&refresh_token={}", refresh1);
    let response = app
        .clone()
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
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "the revoked session must no longer refresh"
    );

    // ...while its same-user sibling still refreshes fine.
    let refresh_body = format!("grant_type=refresh_token&refresh_token={}", refresh2);
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
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the sibling session must survive an access-token revocation"
    );
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
    assert_eq!(
        session.ip_address.as_deref(),
        None,
        "FFI-style in-process requests have no transport peer, so a client-supplied forwarding header is never persisted as a trusted address"
    );
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

// ===========================================================================
// Strict declared-grant parsing (task 02). The declared `grant_type` is the
// sole flow selector; every malformed combination below must fail in the
// OAuth error envelope before either provider port is reached.
// ===========================================================================

/// Regression for the grant-confusion defect: a request declaring
/// `authorization_code` that also carries an `id_token` used to run the
/// direct-assertion path — the code was never redeemed. It must now die at
/// the parse with `invalid_request`, and neither provider method may run.
#[tokio::test]
async fn token_authorization_code_with_id_token_rejected_before_any_provider_call() {
    let (app, _repo, provider) = build_test_app_with_provider_and_grants();

    let response = post_form(
        &app,
        "/token",
        "grant_type=authorization_code&code=test-code&redirect_uri=http://localhost/callback&provider=test&id_token=fake.id.token",
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["error"], "invalid_request");
    assert_eq!(
        json["error_description"],
        "id_token is not a parameter of the authorization_code grant"
    );

    // The decisive assertion: the parse failure happens before the service,
    // so the code was never redeemed AND the direct-assertion path never ran.
    let counts = provider.call_counts();
    assert_eq!(counts.exchange_code, 0, "code must never be redeemed");
    assert_eq!(counts.validate_id_token, 0, "no token may be validated");
}

/// A request declaring `authorization_code` without `redirect_uri` is
/// rejected even when an `id_token` is supplied — the stray credential must
/// not rescue an incomplete authorization-code request.
#[tokio::test]
async fn token_authorization_code_missing_redirect_uri_with_stray_id_token_rejected() {
    let (app, _repo, provider) = build_test_app_with_provider_and_grants();

    let response = post_form(
        &app,
        "/token",
        "grant_type=authorization_code&code=test-code&provider=test&id_token=fake.id.token",
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["error"], "invalid_request");
    assert_eq!(
        json["error_description"],
        "id_token is not a parameter of the authorization_code grant"
    );
    let counts = provider.call_counts();
    assert_eq!(counts.exchange_code, 0);
    assert_eq!(counts.validate_id_token, 0);
}

/// Declaring `id_token` without an `id_token` parameter must be rejected —
/// it must never fall through to a code redemption or reach the provider.
#[tokio::test]
async fn token_id_token_grant_without_id_token_rejected_before_provider_call() {
    let (app, _repo, provider) = build_test_app_with_provider();

    let response = post_form(
        &app,
        "/token",
        "grant_type=id_token&provider=test&code=unused-code",
    )
    .await;

    // Note: the cross-grant `code` field is rejected first by design — the
    // closed-set rule is unconditional, and the rejection still proves no
    // provider call happened for this declared grant.
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["error"], "invalid_request");
    assert_eq!(
        json["error_description"],
        "code is not a parameter of the id_token grant"
    );

    let counts = provider.call_counts();
    assert_eq!(
        counts.exchange_code, 0,
        "must never fall through to code redemption"
    );
    assert_eq!(counts.validate_id_token, 0);
}

/// Pure missing-member case for the id_token grant: no cross-grant fields,
/// but the required `id_token` parameter itself is absent.
#[tokio::test]
async fn token_id_token_grant_missing_id_token_parameter_returns_invalid_request() {
    let (app, _repo, provider) = build_test_app_with_provider();

    let response = post_form(&app, "/token", "grant_type=id_token&provider=test").await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["error"], "invalid_request");
    assert_eq!(
        json["error_description"],
        "missing required parameter: id_token"
    );

    let counts = provider.call_counts();
    assert_eq!(counts.validate_id_token, 0);
}

/// The refresh grant's closed parameter set: `provider` is a member of the
/// two exchange grants only, so carrying it on a refresh is rejected.
#[tokio::test]
async fn token_refresh_grant_with_provider_rejected() {
    let (app, _repo) = build_test_app();

    let response = post_form(
        &app,
        "/token",
        "grant_type=refresh_token&refresh_token=some-token&provider=test",
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["error"], "invalid_request");
    assert_eq!(
        json["error_description"],
        "provider is not a parameter of the refresh_token grant"
    );
}

/// The other exchange-only members are likewise rejected on refresh.
#[tokio::test]
async fn token_refresh_grant_with_code_rejected() {
    let (app, _repo) = build_test_app();

    let response = post_form(
        &app,
        "/token",
        "grant_type=refresh_token&refresh_token=some-token&code=abc",
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["error"], "invalid_request");
    assert_eq!(
        json["error_description"],
        "code is not a parameter of the refresh_token grant"
    );
}

/// A refresh request missing its only required member names it exactly.
#[tokio::test]
async fn token_refresh_grant_missing_refresh_token_parameter_returns_invalid_request() {
    let (app, _repo) = build_test_app();

    let response = post_form(&app, "/token", "grant_type=refresh_token").await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["error"], "invalid_request");
    assert_eq!(
        json["error_description"],
        "missing required parameter: refresh_token"
    );
}

/// An authorization-code request missing each of its three required members
/// in turn names the first offender; the existing missing-code test above
/// already covers the `code` case, this covers `provider` and `redirect_uri`.
#[tokio::test]
async fn token_authorization_code_names_each_missing_required_member() {
    let (app, _repo) = build_test_app();

    let missing_provider =
        post_form(&app, "/token", "grant_type=authorization_code&code=abc").await;
    assert_eq!(missing_provider.status(), StatusCode::BAD_REQUEST);
    let json = body_to_json(missing_provider.into_body()).await;
    assert_eq!(json["error"], "invalid_request");
    assert_eq!(
        json["error_description"],
        "missing required parameter: provider"
    );

    let missing_redirect_uri = post_form(
        &app,
        "/token",
        "grant_type=authorization_code&code=abc&provider=test",
    )
    .await;
    assert_eq!(missing_redirect_uri.status(), StatusCode::BAD_REQUEST);
    let json = body_to_json(missing_redirect_uri.into_body()).await;
    assert_eq!(json["error"], "invalid_request");
    assert_eq!(
        json["error_description"],
        "missing required parameter: redirect_uri"
    );
}

/// A body with no `grant_type` at all must be a `400 invalid_request` in the
/// OAuth envelope — not axum's default `422` plain-text form rejection.
#[tokio::test]
async fn token_missing_grant_type_is_400_invalid_request_envelope_not_422() {
    let (app, _repo, provider) = build_test_app_with_provider();

    let response = post_form(
        &app,
        "/token",
        "code=test-code&redirect_uri=http://localhost/callback&provider=test",
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "application/json",
        "absent grant_type must stay inside the JSON OAuth error envelope"
    );
    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["error"], "invalid_request");
    assert_eq!(
        json["error_description"],
        "missing required parameter: grant_type"
    );

    // Negative space: nothing about the half-parsed body may reach the core.
    let counts = provider.call_counts();
    assert_eq!(counts.exchange_code, 0);
    assert_eq!(counts.validate_id_token, 0);
}

/// A body the form extractor cannot parse at all (wrong content type) is
/// answered in the OAuth envelope too — the endpoint never leaks axum's
/// plain-text 415/422 rejections. Negative space for the extractor's
/// catch-all rejection mapping.
#[tokio::test]
async fn token_non_form_body_is_invalid_request_envelope_not_plain_text_rejection() {
    let (app, _repo, provider) = build_test_app_with_provider();

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/token")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"grant_type":"authorization_code"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "application/json",
        "even an unparseable body must be answered inside the JSON OAuth error envelope"
    );
    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["error"], "invalid_request");
    // Negative space: nothing from the unparseable body reaches the service.
    let counts = provider.call_counts();
    assert_eq!(counts.exchange_code, 0);
    assert_eq!(counts.validate_id_token, 0);
}

/// Present-but-empty `grant_type` counts as present, so it classifies as
/// `unsupported_grant_type` with the stable description.
#[tokio::test]
async fn token_empty_grant_type_returns_unsupported_grant_type() {
    let (app, _repo) = build_test_app();

    let response = post_form(&app, "/token", "grant_type=&provider=test").await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["error"], "unsupported_grant_type");
    assert_eq!(
        json["error_description"],
        "The grant_type parameter is not supported"
    );
}

/// Parameters entirely outside the known set are ignored (RFC 6749 §3.2),
/// unlike known parameters belonging to another grant.
#[tokio::test]
async fn token_unknown_unrelated_parameters_are_ignored() {
    let (app, _repo, provider) = build_test_app_with_provider();

    let response = post_form(
        &app,
        "/token",
        "grant_type=authorization_code&code=test-code&redirect_uri=http://localhost/callback&provider=test&state=xyz&nonce=n-1&custom_future_param=1",
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let json = body_to_json(response.into_body()).await;
    assert!(json.get("access_token").is_some());
    let counts = provider.call_counts();
    assert_eq!(
        counts.exchange_code, 1,
        "the valid exchange still runs once"
    );
}

/// Positive space for the direct-assertion grant through the strict parser.
#[tokio::test]
async fn token_valid_id_token_grant_exchanges_directly() {
    let (app, session_repo, provider) = build_test_app_with_provider_and_grants();

    // The direct grant requires a live nonce (replay protection): mint one
    // and pin claims echoing it, exactly as a real client/provider pair would.
    let minted = post_form(&app, "/nonce", "").await;
    assert_eq!(minted.status(), StatusCode::OK);
    let minted_json = body_to_json(minted.into_body()).await;
    let nonce = minted_json["nonce"].as_str().expect("minted nonce");
    let mut claims = MockIdentityProvider::default_claims();
    claims
        .raw_claims
        .insert("nonce".to_string(), serde_json::json!(nonce));
    provider.set_claims(claims).await;

    let response = post_form(
        &app,
        "/token",
        "grant_type=id_token&id_token=fake.id.token&provider=test",
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let json = body_to_json(response.into_body()).await;
    assert!(json.get("access_token").is_some());
    assert_eq!(json["token_type"], "Bearer");

    // Exactly one direct validation ran, and no code redemption ever did.
    let counts = provider.call_counts();
    assert_eq!(counts.validate_id_token, 1);
    assert_eq!(counts.exchange_code, 0);

    // The full flow completed: the session was stored.
    assert_eq!(session_repo.get_all_sessions().await.len(), 1);
}

/// Valid refresh requests keep their documented shape through the parser.
#[tokio::test]
async fn token_valid_refresh_grant_still_succeeds() {
    let (app, _repo) = build_test_app();

    let exchanged = post_form(
        &app,
        "/token",
        "grant_type=authorization_code&code=test-code&redirect_uri=http://localhost/callback&provider=test",
    )
    .await;
    assert_eq!(exchanged.status(), StatusCode::OK);
    let json = body_to_json(exchanged.into_body()).await;
    let refresh_token = json["refresh_token"].as_str().unwrap().to_string();

    let response = post_form(
        &app,
        "/token",
        &format!("grant_type=refresh_token&refresh_token={refresh_token}"),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let json = body_to_json(response.into_body()).await;
    assert!(json.get("access_token").is_some());
}

// ===========================================================================
// Credential-route cache control (task 03). Every /token and /revoke
// response — success and OAuth error alike — carries `Cache-Control: no-store`
// and `Pragma: no-cache`; /keys, discovery, and /health stay unmarked.
// ===========================================================================

fn assert_no_store_headers_marked(response: &axum::response::Response) {
    assert_eq!(
        response
            .headers()
            .get("cache-control")
            .map(|v| v.to_str().unwrap()),
        Some("no-store"),
        "credential response must carry the exact Cache-Control: no-store directive"
    );
    assert_eq!(
        response
            .headers()
            .get("pragma")
            .map(|v| v.to_str().unwrap()),
        Some("no-cache"),
        "credential response must carry the exact Pragma: no-cache directive"
    );
}

fn assert_no_cache_headers_absent(response: &axum::response::Response) {
    // Negative space: the public metadata endpoints keep their own cacheable
    // policy — a blanket no-store over the whole router would be wrong.
    assert!(
        response.headers().get("cache-control").is_none(),
        "public metadata responses must not be marked no-store"
    );
    assert!(
        response.headers().get("pragma").is_none(),
        "public metadata responses must not be marked no-cache"
    );
}

#[tokio::test]
async fn token_success_response_carries_no_store_headers() {
    let (app, _repo) = build_test_app();

    let response = post_form(
        &app,
        "/token",
        "grant_type=authorization_code&code=test-code&redirect_uri=http://localhost/callback&provider=test",
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_no_store_headers_marked(&response);
}

#[tokio::test]
async fn token_unsupported_grant_error_response_carries_no_store_headers() {
    let (app, _repo) = build_test_app();

    let response = post_form(&app, "/token", "grant_type=client_credentials").await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_no_store_headers_marked(&response);
    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["error"], "unsupported_grant_type");
}

/// The `ApiError::Domain` error path (rendered by `into_response` inside the
/// handler's result) must be covered too — an invalid_request envelope is
/// credential-adjacent and must not be storable either.
#[tokio::test]
async fn token_invalid_request_error_response_carries_no_store_headers() {
    let (app, _repo) = build_test_app();

    let response = post_form(
        &app,
        "/token",
        "grant_type=authorization_code&code=test-code&redirect_uri=http://localhost/callback&provider=test&refresh_token=rt-1",
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_no_store_headers_marked(&response);
    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["error"], "invalid_request");
}

#[tokio::test]
async fn revoke_response_carries_no_store_headers() {
    let (app, _repo) = build_test_app();

    let response = post_form(
        &app,
        "/revoke",
        "token=some-refresh-token&token_type_hint=refresh_token",
    )
    .await;

    // /revoke sits in the shared credential route group by explicit source-
    // spec decision even though RFC 7009 imposes no cache requirement.
    assert_eq!(response.status(), StatusCode::OK);
    assert_no_store_headers_marked(&response);

    // The 503 store-failure path on the same route is marked as well. A real
    // session must exist first: an unknown token short-circuits to 200
    // without touching the failing store call (RFC 7009 idempotency).
    let (app, session_repo) = build_test_app();
    let exchanged = post_form(
        &app,
        "/token",
        "grant_type=authorization_code&code=test-code&redirect_uri=http://localhost/callback&provider=test",
    )
    .await;
    assert_eq!(exchanged.status(), StatusCode::OK);
    let json = body_to_json(exchanged.into_body()).await;
    let refresh_token = json["refresh_token"].as_str().unwrap().to_string();

    session_repo.set_session_fail_mode(true).await;
    let failing = post_form(
        &app,
        "/revoke",
        &format!("token={refresh_token}&token_type_hint=refresh_token"),
    )
    .await;
    assert_eq!(failing.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_no_store_headers_marked(&failing);
}

#[tokio::test]
async fn keys_response_has_no_cache_directives() {
    let (app, _repo) = build_test_app();

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
    assert_no_cache_headers_absent(&response);
}

#[tokio::test]
async fn discovery_response_has_no_cache_directives() {
    let (app, _repo) = build_test_app();

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
    assert_no_cache_headers_absent(&response);
}

// ---------------------------------------------------------------------------
// 10. With grants.id_token disabled (the compiled default), an id_token field
// is rejected as unsupported_grant_type whatever grant_type declares — the
// handler-level gate, shared with Lambda and FFI through this same function.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn token_disabled_grant_rejects_id_token_grant_type() {
    let (app, _session_repo) = build_test_app();

    let body = "grant_type=id_token&id_token=fake.jwt.value&provider=test";

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

#[tokio::test]
async fn token_disabled_grant_rejects_id_token_field_under_authorization_code() {
    let (app, _session_repo) = build_test_app();

    // Field-presence branch selection cannot evade the switch: the id_token
    // field alone triggers the rejection under a different declared grant.
    let body = "grant_type=authorization_code&code=test-code&redirect_uri=http://localhost/callback&provider=test&id_token=fake.jwt.value";

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
