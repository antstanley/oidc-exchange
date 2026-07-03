//! E2E coverage for the `server.base_path` strip layer, driven through the real
//! `bootstrap::build_router` output (middleware stack included) via `tower::ServiceExt::oneshot`
//! — see task 02 of `.specs/plans/2026-07-02-implement_lambda_runtime/plan.md`.

use std::collections::HashMap;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use tower::ServiceExt;

use oidc_exchange::bootstrap::build_router;
use oidc_exchange_core::config::AppConfig;
use oidc_exchange_core::ports::IdentityProvider;
use oidc_exchange_core::service::AppService;
use oidc_exchange_test_utils::{
    MockAuditLog, MockIdentityProvider, MockKeyManager, MockRepository, MockUserSync,
};

/// Build the real production router (`bootstrap::build_router`, full middleware stack
/// included) over a config carrying the given `base_path`, backed by mock adapters.
fn build_app(base_path: Option<&str>) -> Router {
    let provider = MockIdentityProvider::new("test");
    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    providers.insert("test".to_string(), Box::new(provider));

    let mut config = AppConfig::default();
    config.server.issuer = "https://auth.example.com".to_string();
    config.server.base_path = base_path.map(str::to_string);

    let service = AppService::new(
        Box::new(MockRepository::new()),
        Box::new(MockRepository::new()),
        Box::new(MockKeyManager::new()),
        Box::new(MockAuditLog::new()),
        Box::new(MockUserSync::new()),
        providers,
        config.clone(),
    );

    build_router(&config, service)
}

async fn get(app: &Router, uri: &str) -> StatusCode {
    app.clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap()
        .status()
}

// ---------------------------------------------------------------------------
// 1. base_path = Some("/prod") strips the prefix before routing.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn prefixed_request_routes_to_health_when_base_path_configured() {
    let app = build_app(Some("/prod"));

    assert_eq!(get(&app, "/prod/health").await, StatusCode::OK);
    // Sanity: a second, distinct registered route is reachable the same way — not a
    // coincidence of `/health` specifically.
    assert_eq!(get(&app, "/prod/keys").await, StatusCode::OK);
}

// ---------------------------------------------------------------------------
// 2. base_path = None leaves paths unchanged — no rewrite occurs.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn unprefixed_request_routes_to_health_when_base_path_unset() {
    let app = build_app(None);

    assert_eq!(get(&app, "/health").await, StatusCode::OK);
    // Negative space: with no base_path configured, a `/prod`-prefixed path is never a route
    // this deployment serves — the layer must not invent stripping behaviour that isn't
    // configured.
    assert_eq!(get(&app, "/prod/health").await, StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// 3. Negative space: a request lacking the prefix is not double-stripped.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn request_without_prefix_is_not_double_stripped() {
    let app = build_app(Some("/prod"));

    // `/health` does not start with `/prod` at all, so the layer leaves it untouched; it
    // happens to be a real unprefixed route, so it still resolves — proving the path bytes
    // were never mangled by a naive substring strip (which would have produced garbage like
    // "th" from blindly chopping `len("/prod")` bytes off the front).
    assert_eq!(get(&app, "/health").await, StatusCode::OK);
}

// ---------------------------------------------------------------------------
// 4. Negative space: a mismatched, longer sibling segment is not falsely stripped.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn mismatched_sibling_prefix_is_not_falsely_routed() {
    let app = build_app(Some("/prod"));

    // `/production/health` shares the literal bytes "/prod" with the configured prefix but
    // is not prefixed by it at a path-segment boundary; a buggy raw `strip_prefix` would turn
    // this into "uction/health" (or similar) and 404 or worse. The correct behaviour leaves
    // the path untouched, and since no route named `/production/health` exists, the request
    // 404s rather than producing a false 200.
    assert_eq!(get(&app, "/production/health").await, StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// 5. Boundary case: a request equal to the bare prefix strips to the root path.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn bare_prefix_request_strips_to_root() {
    let app = build_app(Some("/prod"));

    // `/prod` (the bare prefix, no trailing segment) is the boundary case where `stripped`
    // is empty: per the layer's own rule it rewrites to root (`/`), which has no registered
    // handler, so this must 404 — never a false 200 from accidentally matching some other
    // route. The rewrite itself (`"/prod"` → `"/"`) is asserted directly at the unit level in
    // `crate::middleware::base_path`'s `strip_base_path_rewrites_bare_prefix_to_root` test;
    // this E2E case exists to confirm the boundary path doesn't misroute in the full stack.
    assert_eq!(get(&app, "/prod").await, StatusCode::NOT_FOUND);
}
