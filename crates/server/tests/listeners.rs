//! E2E coverage for the task-04 listener split: the role × `internal_api.enabled`
//! matrix, driven through the real `bootstrap::build_routers` output (full
//! middleware stacks included) via `tower::ServiceExt::oneshot`.
//!
//! The property under test is that the two planes are *disjoint*: `/token` and
//! friends are absent from every admin router (404 from routing, never 401/200),
//! and `/internal/*` is absent from every public router. A merged router would
//! fail these negative assertions with a 401 or a 200 instead of a 404.

use std::collections::HashMap;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use tower::ServiceExt;

use oidc_exchange::bootstrap::{build_routers, Routers};
use oidc_exchange_core::config::{DEFAULT_INTERNAL_API_HOST, DEFAULT_INTERNAL_API_PORT};
use oidc_exchange_core::ports::IdentityProvider;
use oidc_exchange_core::service::AppService;
use oidc_exchange_test_utils::{
    MockAuditLog, MockIdentityProvider, MockKeyManager, MockRateLimiter, MockRepository,
    MockUserSync,
};

const TEST_SECRET: &str = "test-internal-secret-listeners-e2e";

/// Build both planes through the production entry point over mock adapters.
fn build_planes(role: &str, internal_enabled: bool) -> Routers {
    let provider = MockIdentityProvider::new("test");
    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    providers.insert("test".to_string(), Box::new(provider));

    let mut raw: oidc_exchange_core::config::RawConfig =
        toml::from_str(include_str!("../../../config/default.toml"))
            .expect("default test config is valid");
    raw.server.issuer = "https://auth.example.com".to_string();
    raw.server.role = role.to_string();
    raw.internal_api.enabled = internal_enabled;
    raw.internal_api.auth_methods = vec!["shared_secret".to_string()];
    raw.internal_api.shared_secret = Some(TEST_SECRET.to_string());
    let config = oidc_exchange_core::config::Config::resolve(raw)
        .expect("listener-matrix test config resolves");

    let service = AppService::new(
        Box::new(MockRepository::new()),
        Box::new(MockRepository::new()),
        Box::new(MockKeyManager::new()),
        Box::new(MockAuditLog::new()),
        Box::new(MockUserSync::new()),
        Box::new(MockRateLimiter::new()),
        providers,
        config.clone(),
    );

    let routers = build_routers(&config, service)
        .expect("the listener-matrix test configs always build routers");
    assert!(
        !routers.is_empty(),
        "a validated role ({role}) must produce at least one router"
    );
    routers
}

async fn get(app: &Router, uri: &str, bearer: Option<&str>) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder().method("GET").uri(uri);
    if let Some(token) = bearer {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();

    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("response body always collects")
        .to_bytes();
    let json = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, json)
}

// ---------------------------------------------------------------------------
// 1. role = "exchange" (the default): public plane only.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn exchange_role_binds_public_only() {
    let routers = build_planes("exchange", true);

    let public = routers.public.expect("exchange binds the public plane");
    assert!(
        routers.admin.is_none(),
        "role = \"exchange\" must not produce an admin router at all"
    );

    let (status, _) = get(&public, "/health", None).await;
    assert_eq!(status, StatusCode::OK, "public health must be served");
    let (status, _) = get(&public, "/keys", None).await;
    assert_eq!(status, StatusCode::OK, "JWKS must be publicly served");

    // Negative space: the internal surface does not exist on this plane — even
    // presenting the valid credential changes nothing because there is no route.
    // An empty response body is itself evidence: axum's routing-level 404 carries
    // no body at all, whereas an auth rejection would render a JSON envelope.
    let (status, body) = get(&public, "/internal/stats", Some(TEST_SECRET)).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "/internal/stats must be absent from the public plane regardless of credential"
    );
    assert!(
        body.is_null(),
        "a routing-level miss must carry no handler-rendered body, got {body}"
    );
}

/// The final role × flag combination (completing the 2×3 matrix): a
/// disabled internal API under the exchange role changes nothing — no admin
/// router exists and the public surface is intact.
#[tokio::test]
async fn exchange_role_disabled_binds_the_same_public_only_plane() {
    let routers = build_planes("exchange", false);

    let public = routers.public.expect("exchange binds the public plane");
    assert!(
        routers.admin.is_none(),
        "role = \"exchange\" never binds an admin router, flag on or off"
    );

    let (status, _) = get(&public, "/health", None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = get(&public, "/keys", None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = get(&public, "/internal/stats", Some(TEST_SECRET)).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "the internal surface stays absent with the flag off"
    );
}

// ---------------------------------------------------------------------------
// 2. role = "admin", enabled: admin plane only, no exchange routes.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn admin_role_enabled_binds_admin_only_without_public_routes() {
    let routers = build_planes("admin", true);

    let admin = routers.admin.expect("admin binds the admin plane");
    assert!(
        routers.public.is_none(),
        "role = \"admin\" must not produce a public router at all"
    );

    let (status, _) = get(&admin, "/health", None).await;
    assert_eq!(status, StatusCode::OK, "admin health must be served");
    let (status, _) = get(&admin, "/internal/stats", Some(TEST_SECRET)).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the valid operator credential reaches the internal handler"
    );

    for path in ["/token", "/revoke", "/keys"] {
        let (status, _) = get(&admin, path, None).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "{path} must be absent from the admin listener — network policy can expose \
             this socket without exposing any exchange endpoint"
        );
    }
}

// ---------------------------------------------------------------------------
// 3. role = "all", enabled: BOTH planes bind, each disjoint from the other.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn all_role_binds_two_disjoint_listeners() {
    let routers = build_planes("all", true);

    let public = routers.public.expect("all binds the public plane");
    let admin = routers.admin.expect("all binds the admin plane");
    // Two distinct sockets: the defaults are loopback + one above the public
    // port, asserted here so the test fails loudly if either default drifts.
    assert_eq!(
        (DEFAULT_INTERNAL_API_HOST, DEFAULT_INTERNAL_API_PORT),
        ("127.0.0.1", 8081),
        "the admin listener defaults to loopback:8081, distinct from the public 8080"
    );

    // Public plane: exchange routes present, internal routes absent.
    let (status, _) = get(&public, "/health", None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = get(&public, "/internal/users", Some(TEST_SECRET)).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "/internal/* must be absent from the public router — proving no merge happened"
    );
    assert!(body.is_null(), "routing miss must have no body, got {body}");

    // Admin plane: internal routes behind auth, exchange routes absent.
    let (status, _) = get(&admin, "/internal/stats", Some(TEST_SECRET)).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = get(&admin, "/token", Some(TEST_SECRET)).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "/token must be absent from the admin router even with a valid credential"
    );
    assert!(
        body.is_null(),
        "the miss must be a routing 404 with no body, not an auth failure or an accidental hit"
    );
}

// ---------------------------------------------------------------------------
// 4. role = "all", disabled: public plane normal; admin plane is /health only.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn all_role_disabled_serves_health_only_on_the_admin_plane() {
    let routers = build_planes("all", false);

    let public = routers.public.expect("all binds the public plane");
    let admin = routers
        .admin
        .expect("all still binds a health-only admin plane");

    let (status, _) = get(&public, "/health", None).await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = get(&admin, "/health", None).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the admin listener stays probeable while the internal API is disabled"
    );
    let (status, _) = get(&admin, "/internal/stats", Some(TEST_SECRET)).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "with the flag off there are no internal routes anywhere — 404 by routing"
    );
}

// ---------------------------------------------------------------------------
// 5. role = "admin", disabled: observable but inert.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn admin_role_disabled_is_observable_but_inert() {
    let routers = build_planes("admin", false);

    let admin = routers
        .admin
        .expect("admin binds its plane when disabled too");
    assert!(routers.public.is_none());

    let (status, _) = get(&admin, "/health", None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = get(&admin, "/internal/users", Some(TEST_SECRET)).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "disabled means unmounted: no internal route exists to authenticate against"
    );
}
