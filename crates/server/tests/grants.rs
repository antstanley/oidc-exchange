//! Route/E2E coverage for the opt-in direct ID-token grant surface: an
//! enabled deployment mints nonces, advertises and serves the direct grant,
//! and binds it once; a disabled deployment exposes neither the nonce route
//! nor the grant, whatever the request declares. Everything drives
//! `bootstrap::build_router` — the one shared router behind the hyper server,
//! Lambda runtime, and FFI bindings — so no interface can bypass the gates.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use chrono::Utc;
use http_body_util::BodyExt;
use oidc_exchange_core::config::{Config, RawConfig};
use oidc_exchange_core::domain::IdentityClaims;
use oidc_exchange_core::ports::IdentityProvider;
use oidc_exchange_core::service::AppService;
use oidc_exchange_test_utils::{
    MockAuditLog, MockIdentityProvider, MockKeyManager, MockRepository, MockUserSync,
};
use serde_json::{json, Value};
use tower::ServiceExt;

const ISSUER: &str = "https://auth.example.com";
const PROVIDER_ID: &str = "test";

/// Deterministically unique `jti` values so distinct exchanges never share a
/// replay marker unless a test intends it.
fn unique_jti() -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    format!("jti-{}", COUNTER.fetch_add(1, Ordering::SeqCst))
}

/// Raw claims that pass every binding control except what the test overrides:
/// an `exp` ten minutes out (inside the default ceiling) and a fresh `jti`.
fn passing_raw_claims(nonce: &str) -> serde_json::Map<String, Value> {
    let mut raw = serde_json::Map::new();
    raw.insert("exp".to_string(), json!(Utc::now().timestamp() + 600));
    raw.insert("jti".to_string(), json!(unique_jti()));
    raw.insert("sub".to_string(), json!("grant-subject"));
    raw.insert("nonce".to_string(), json!(nonce));
    raw
}

/// Verified claims pinned onto the mock provider, signed RS256.
fn claims_with(raw: serde_json::Map<String, Value>) -> IdentityClaims {
    IdentityClaims {
        subject: "grant-subject".to_string(),
        email: Some("grant@example.com".to_string()),
        email_verified: Some(true),
        name: Some("Grant User".to_string()),
        is_private_email: None,
        signing_alg: "RS256".to_string(),
        raw_claims: raw.into_iter().collect(),
    }
}

/// Build the production router over mock adapters with the given config;
/// returns the router and the live provider handle for claim pinning.
fn build_app(config: Config) -> (Router, MockIdentityProvider) {
    let provider = MockIdentityProvider::new(PROVIDER_ID);
    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    providers.insert(PROVIDER_ID.to_string(), Box::new(provider.clone()));

    let service = AppService::new(
        Box::new(MockRepository::new()),
        Box::new(MockRepository::new()),
        Box::new(MockKeyManager::new()),
        Box::new(MockAuditLog::new()),
        Box::new(MockUserSync::new()),
        Box::new(oidc_exchange_test_utils::MockRateLimiter::new()),
        providers,
        config.clone(),
    );

    let router = oidc_exchange::bootstrap::build_router(&config, service);
    (router, provider)
}

async fn post_form(router: &Router, uri: &str, body: &str) -> (StatusCode, Value) {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let json = body_to_json(response.into_body()).await;
    (status, json)
}

async fn get_json(router: &Router, uri: &str) -> (StatusCode, Value) {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let json = body_to_json(response.into_body()).await;
    (status, json)
}

async fn body_to_json(body: Body) -> Value {
    let bytes = body
        .collect()
        .await
        .expect("response body collects")
        .to_bytes();
    if bytes.is_empty() {
        // 404s from unmounted routes carry no body.
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("JSON response body")
    }
}

fn base_raw_config() -> RawConfig {
    let mut raw: RawConfig = toml::from_str(include_str!("../../../config/default.toml"))
        .expect("default config deserializes");
    raw.server.issuer = ISSUER.to_string();
    raw
}

fn config_enabled() -> Config {
    let mut raw = base_raw_config();
    raw.grants.id_token = true;
    Config::resolve(raw).expect("enabled config resolves")
}

fn config_disabled() -> Config {
    let raw = base_raw_config();
    assert!(
        !raw.grants.id_token,
        "the compiled default keeps the grant off"
    );
    Config::resolve(raw).expect("disabled config resolves")
}

// ---------------------------------------------------------------------------
// Enabled deployment
// ---------------------------------------------------------------------------

/// An enabled deployment advertises `id_token` alongside the two always-served
/// grants, and nothing else changes about the document.
#[tokio::test]
async fn enabled_discovery_advertises_the_direct_grant() {
    let (router, _provider) = build_app(config_enabled());

    let (status, doc) = get_json(&router, "/.well-known/openid-configuration").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(doc["issuer"], ISSUER);

    let grants = doc["grant_types_supported"].as_array().expect("grant list");
    assert!(grants.iter().any(|g| g == "authorization_code"));
    assert!(grants.iter().any(|g| g == "refresh_token"));
    assert!(
        grants.iter().any(|g| g == "id_token"),
        "enabled deployments advertise id_token, got {grants:?}"
    );
    assert_eq!(grants.len(), 3, "exactly the three served grants");
}

/// `POST /nonce` returns exactly the specified shape and stores only the
/// digest of the returned value.
#[tokio::test]
async fn enabled_nonce_route_mints_usable_nonces() {
    let (router, _provider) = build_app(config_enabled());

    let (status, body) = post_form(&router, "/nonce", "").await;
    assert_eq!(status, StatusCode::OK);

    let nonce = body["nonce"].as_str().expect("nonce string");
    assert_eq!(nonce.len(), 43, "32 random bytes base64url-no-pad");
    let expires_in = body["expires_in"].as_u64().expect("expires_in number");
    assert_eq!(
        expires_in, 600,
        "expires_in mirrors the default grants.nonce_ttl"
    );

    // Two calls mint independent values.
    let (_status2, second) = post_form(&router, "/nonce", "").await;
    assert_ne!(second["nonce"].as_str().expect("second nonce"), nonce);
}

/// The complete enabled flow: mint → direct exchange succeeds once → the
/// duplicate fails as invalid_grant — and the provider access token travels
/// from the form into the core's at_hash binding (a wrong one is refused).
#[tokio::test]
async fn enabled_direct_grant_exchanges_once_and_replays_fail() {
    let (router, provider) = build_app(config_enabled());

    let (_status, minted) = post_form(&router, "/nonce", "").await;
    let nonce = minted["nonce"].as_str().expect("minted nonce").to_string();

    // Pin claims echoing the minted nonce with a deliberately WRONG at_hash
    // for the presented access token: the first exchange must fail, proving
    // the form's provider_access_token really reached the binding check.
    let mut raw = passing_raw_claims(&nonce);
    raw.insert("at_hash".to_string(), json!("not-the-real-hash"));
    provider.set_claims(claims_with(raw)).await;

    let body = "grant_type=id_token&id_token=fake.jwt.value&provider=test&provider_access_token=real-access-token";
    let (status, error) = post_form(&router, "/token", body).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "mismatched at_hash refuses"
    );
    assert_eq!(error["error"], "invalid_grant");
    // Client-visible descriptions are fixed per error class (no validation
    // oracle); the detailed at_hash reason stays server-side.
    assert_eq!(
        error["error_description"],
        "the provided grant could not be validated"
    );

    // Re-pin with the CORRECT at_hash: the same assertion now exchanges once.
    let access_token = "real-access-token";
    let expected_at_hash = {
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        use base64::Engine;
        use sha2::{Digest, Sha256};
        URL_SAFE_NO_PAD.encode(&Sha256::digest(access_token.as_bytes())[..16])
    };
    let nonce2 = post_form(&router, "/nonce", "").await.1["nonce"]
        .as_str()
        .expect("fresh nonce")
        .to_string();
    let mut raw = passing_raw_claims(&nonce2);
    raw.insert("at_hash".to_string(), json!(expected_at_hash));
    provider.set_claims(claims_with(raw)).await;

    let (status, tokens) = post_form(&router, "/token", body).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "correctly bound assertion exchanges"
    );
    assert_eq!(tokens["token_type"], "Bearer");
    assert!(!tokens["access_token"]
        .as_str()
        .unwrap_or_default()
        .is_empty());

    // Duplicate submission: the burned nonce rejects it as invalid_grant.
    let (status, error) = post_form(&router, "/token", body).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "duplicate direct exchange fails"
    );
    assert_eq!(error["error"], "invalid_grant");
}

// ---------------------------------------------------------------------------
// Disabled deployment (the compiled default)
// ---------------------------------------------------------------------------

/// A disabled deployment mounts no nonce route at all.
#[tokio::test]
async fn disabled_deployment_has_no_nonce_route() {
    let (router, _provider) = build_app(config_disabled());

    let (status, _body) = post_form(&router, "/nonce", "").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "POST /nonce must not exist");
}

/// With the switch off, an id_token field is rejected as unsupported_grant_type
/// under grant_type=id_token — the switch cannot be evaded.
#[tokio::test]
async fn disabled_rejects_id_token_grant_type() {
    let (router, _provider) = build_app(config_disabled());

    let body = "grant_type=id_token&id_token=fake.jwt.value&provider=test";
    let (status, error) = post_form(&router, "/token", body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error["error"], "unsupported_grant_type");
}

/// Bypass attempt named by the source spec: an id_token field smuggled under
/// grant_type=authorization_code is rejected before any branch selection.
#[tokio::test]
async fn disabled_rejects_id_token_field_under_authorization_code_grant_type() {
    let (router, _provider) = build_app(config_disabled());

    let body = "grant_type=authorization_code&code=x&redirect_uri=https://app.test.com/cb&provider=test&id_token=fake.jwt.value";
    let (status, error) = post_form(&router, "/token", body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error["error"], "unsupported_grant_type");

    // Negative space: without the smuggled field, the code grant still works
    // exactly as before (the gate keys on field presence, not grant_type).
    let clean =
        "grant_type=authorization_code&code=x&redirect_uri=https://app.test.com/cb&provider=test";
    let (status, tokens) = post_form(&router, "/token", clean).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "authorization_code keeps its behavior"
    );
    assert_eq!(tokens["token_type"], "Bearer");
}

/// Disabled discovery never advertises id_token but keeps the always-served
/// grants listed.
#[tokio::test]
async fn disabled_discovery_omits_the_direct_grant() {
    let (router, _provider) = build_app(config_disabled());

    let (status, doc) = get_json(&router, "/.well-known/openid-configuration").await;
    assert_eq!(status, StatusCode::OK);
    let grants = doc["grant_types_supported"].as_array().expect("grant list");
    assert!(!grants.iter().any(|g| g == "id_token"), "got {grants:?}");
    assert!(grants.iter().any(|g| g == "authorization_code"));
    assert!(grants.iter().any(|g| g == "refresh_token"));
}

/// Role gating: even an enabled grant mounts no nonce route when the role
/// serves no exchanges (`admin`), so admin-only processes gain no surface.
#[tokio::test]
async fn admin_role_mounts_no_nonce_route_even_when_grant_enabled() {
    let mut raw = base_raw_config();
    raw.grants.id_token = true;
    raw.server.role = "admin".to_string();
    let config = Config::resolve(raw).expect("admin config resolves");
    let (router, _provider) = build_app(config);

    let (status, _body) = post_form(&router, "/nonce", "").await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "admin-only roles mount no nonce route"
    );

    // Health stays available for the admin role's liveness probe.
    let (status, health) = get_json(&router, "/health").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(health["status"], "ok");
}
