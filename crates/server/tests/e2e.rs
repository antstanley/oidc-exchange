use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use axum::Router;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use http_body_util::BodyExt;
use serde_json::json;
use tower::ServiceExt;

use oidc_exchange::bootstrap::{build_router_with_rate_limiter, build_routers, Routers};
use oidc_exchange::middleware::throttle::{FixedWindowRateLimiter, RateLimitBudgets, TestClock};
use oidc_exchange_core::config::{Config, RawConfig};
use oidc_exchange_core::domain::{
    AuditEventType, ClientAddrSource, RateLimitDecision, RateLimitKey,
};
use oidc_exchange_core::ports::IdentityProvider;
use oidc_exchange_core::service::AppService;
use oidc_exchange_test_utils::{
    MockAuditLog, MockIdentityProvider, MockKeyManager, MockRateLimiter, MockRepository,
    MockUserSync,
};

const TEST_SECRET: &str = "test-internal-secret-e2e-0123456789ab";

fn test_config(registration_mode: &str) -> Config {
    let mut raw_config: RawConfig = toml::from_str(include_str!("../../../config/default.toml"))
        .expect("default test config is valid");
    raw_config.server.issuer = "https://auth.example.com".to_string();
    raw_config.server.role = "all".to_string();
    raw_config.registration.mode = registration_mode.to_string();
    raw_config.internal_api.enabled = true;
    raw_config.internal_api.auth_methods = vec!["shared_secret".to_string()];
    raw_config.internal_api.shared_secret = Some(TEST_SECRET.to_string());
    Config::resolve(raw_config).expect("test config should resolve")
}

fn base_raw() -> RawConfig {
    toml::from_str(include_str!("../../../config/default.toml"))
        .expect("default test config is valid")
}

/// The two production planes (`bootstrap::build_routers`, role = "all") over
/// mock adapters — the same disjoint routers a real `role = "all"` process
/// serves on separate sockets. E2E flows that cross planes send each request
/// to its own plane, so no test ever relies on a merged surface.
fn build_e2e_planes_with_config(config: Config) -> Routers {
    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    providers.insert(
        "test".to_string(),
        Box::new(MockIdentityProvider::new("test")),
    );

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

    let routers =
        build_routers(&config, service).expect("the e2e test config always builds routers");
    assert!(routers.public.is_some() && routers.admin.is_some());
    routers
}

/// The default e2e planes: internal API enabled behind the shared secret.
fn build_e2e_planes() -> Routers {
    build_e2e_planes_with_config(test_config("open"))
}

async fn body_to_json(body: Body) -> serde_json::Value {
    let bytes = body.collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

fn build_production_audit_router(
    peer: SocketAddr,
    forwarded: Option<&str>,
    audit_log: MockAuditLog,
    public_limiter: MockRateLimiter,
) -> (Router, Request<Body>) {
    let provider = MockIdentityProvider::new("test");
    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    providers.insert("test".to_string(), Box::new(provider));

    let mut raw = base_raw();
    raw.server.role = "exchange".to_string();
    raw.server.issuer = "https://auth.example.com".to_string();
    raw.server.trusted_proxies = vec!["10.0.0.0/8".to_string()];
    raw.server.trusted_proxy_hops = 1;
    let config = Config::resolve(raw).expect("test config resolves");
    let service = AppService::new(
        Box::new(MockRepository::new()),
        Box::new(MockRepository::new()),
        Box::new(MockKeyManager::new()),
        Box::new(audit_log),
        Box::new(MockUserSync::new()),
        Box::new(oidc_exchange_adapters::noop::NoopRateLimiter::new()),
        providers,
        config.clone(),
    );
    let app = build_router_with_rate_limiter(&config, service, Arc::new(public_limiter));
    let mut request = Request::builder()
        .method("POST")
        .uri("/token")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(
            "grant_type=authorization_code&code=test-code&redirect_uri=http://localhost/callback&provider=test",
        ))
        .unwrap();
    if let Some(forwarded) = forwarded {
        request.headers_mut().insert(
            "x-forwarded-for",
            forwarded.parse().expect("valid forwarded header"),
        );
    }
    request.extensions_mut().insert(ConnectInfo(peer));
    (app, request)
}

fn build_throttled_router(
    mut raw: RawConfig,
    provider: MockIdentityProvider,
    service_limiter: MockRateLimiter,
    public_limiter: MockRateLimiter,
) -> Router {
    raw.server.role = "exchange".to_string();
    raw.server.issuer = "https://auth.example.com".to_string();
    let config = Config::resolve(raw).expect("test config resolves");
    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    providers.insert("test".to_string(), Box::new(provider));
    let service = AppService::new(
        Box::new(MockRepository::new()),
        Box::new(MockRepository::new()),
        Box::new(MockKeyManager::new()),
        Box::new(MockAuditLog::new()),
        Box::new(MockUserSync::new()),
        Box::new(service_limiter),
        providers,
        config.clone(),
    );
    build_router_with_rate_limiter(&config, service, Arc::new(public_limiter))
}

fn token_request(body: &str) -> Request<Body> {
    let mut request = Request::builder()
        .method("POST")
        .uri("/token")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(body.to_owned()))
        .unwrap();
    request
        .extensions_mut()
        .insert(oidc_exchange::middleware::audit_context::AuditContext {
            client_addr: oidc_exchange_core::domain::ClientAddr::Peer("192.0.2.1".parse().unwrap()),
            user_agent: None,
            device_id: None,
        });
    request
}

#[tokio::test]
async fn production_access_log_is_correlated_to_request_id_and_provenance() {
    let audit_log = MockAuditLog::new();
    let public_limiter = MockRateLimiter::new();
    let (app, mut request) = build_production_audit_router(
        "10.0.0.9:443".parse().unwrap(),
        Some("203.0.113.4"),
        audit_log,
        public_limiter,
    );
    request
        .headers_mut()
        .insert("x-request-id", "request-id-audit-log".parse().unwrap());
    let response = app.oneshot(request).await.expect("router response");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(oidc_exchange::middleware::access_log::ACCESS_LOG_REQUEST_ID_HEADER)
            .and_then(|value| value.to_str().ok()),
        Some("request-id-audit-log")
    );
    assert_eq!(
        response
            .headers()
            .get(oidc_exchange::middleware::access_log::ACCESS_LOG_CLIENT_ADDR_SOURCE_HEADER)
            .and_then(|value| value.to_str().ok()),
        Some("forwarded")
    );
}

#[tokio::test]
async fn production_router_uses_observed_peer_and_trusted_forwarding_for_audit_and_throttle() {
    let cases = [
        (
            "peer",
            "198.51.100.9:443",
            Some("203.0.113.4"),
            "peer",
            "198.51.100.9",
        ),
        (
            "trusted_forwarded",
            "10.0.0.9:443",
            Some("203.0.113.4"),
            "forwarded",
            "203.0.113.4",
        ),
        (
            "forged_forwarded",
            "198.51.100.9:443",
            Some("203.0.113.4"),
            "peer",
            "198.51.100.9",
        ),
        (
            "missing_forwarded",
            "10.0.0.9:443",
            None,
            "peer",
            "10.0.0.9",
        ),
    ];

    for (name, peer, forwarded, source, address) in cases {
        let audit_log = MockAuditLog::new();
        let public_limiter = MockRateLimiter::new();
        let (app, request) = build_production_audit_router(
            peer.parse().expect("valid peer"),
            forwarded,
            audit_log.clone(),
            public_limiter.clone(),
        );

        let response = app.oneshot(request).await.expect("router response");
        assert_eq!(response.status(), StatusCode::OK, "{name}");
        assert_eq!(
            response
                .headers()
                .get(oidc_exchange::middleware::access_log::ACCESS_LOG_CLIENT_ADDR_SOURCE_HEADER)
                .and_then(|value| value.to_str().ok()),
            Some(source),
            "{name}"
        );

        let events = audit_log.events().await;
        assert!(!events.is_empty(), "{name}");
        assert!(
            events
                .iter()
                .all(|event| event.ip_address.as_deref() == Some(address)),
            "{name}: {events:#?}"
        );
        assert_eq!(
            public_limiter.keys().await,
            vec![RateLimitKey::ClientAddr(
                address.parse().expect("valid address")
            )],
            "{name}"
        );
    }
}

/// S7: the terminal `/token` audit event records the middleware's *resolved*
/// `ip_address_source` (`peer`/`forwarded`/`unknown`) rather than the flattened
/// `asserted` the core flow used to manufacture. Before the fix, every
/// core-flow event carried `ip_address_source = "asserted"` regardless of how
/// the middleware learned the address; this drives the production router end to
/// end and inspects the emitted `TokenExchange` event's provenance directly.
#[tokio::test]
async fn production_flow_audit_events_record_resolved_provenance_source() {
    // (name, peer, forwarded, strip_connect_info, expected source)
    let cases = [
        (
            "peer",
            "198.51.100.9:443",
            Some("203.0.113.4"),
            false,
            ClientAddrSource::Peer,
        ),
        (
            "trusted_forwarded",
            "10.0.0.9:443",
            Some("203.0.113.4"),
            false,
            ClientAddrSource::Forwarded,
        ),
        (
            "no_peer",
            "10.0.0.9:443",
            None,
            true,
            ClientAddrSource::Unknown,
        ),
    ];

    for (name, peer, forwarded, strip_connect_info, expected_source) in cases {
        let audit_log = MockAuditLog::new();
        let public_limiter = MockRateLimiter::new();
        let (app, mut request) = build_production_audit_router(
            peer.parse().expect("valid peer"),
            forwarded,
            audit_log.clone(),
            public_limiter,
        );
        if strip_connect_info {
            // No server-established transport peer: provenance must resolve to
            // Unknown, not be manufactured as Asserted.
            request.extensions_mut().remove::<ConnectInfo<SocketAddr>>();
        }

        let response = app.oneshot(request).await.expect("router response");
        assert_eq!(response.status(), StatusCode::OK, "{name}");

        let events = audit_log.events().await;
        assert!(
            !events.is_empty(),
            "{name}: the flow must emit audit events"
        );
        // No core-flow event may carry the flattened `asserted` provenance any
        // more — that is precisely the S7 regression.
        assert!(
            events
                .iter()
                .all(|event| event.ip_address_source != ClientAddrSource::Asserted),
            "{name}: no core-flow event may record asserted provenance: {events:#?}"
        );
        let terminal = events
            .iter()
            .find(|event| event.event_type == AuditEventType::TokenExchange)
            .unwrap_or_else(|| panic!("{name}: a terminal TokenExchange event must be emitted"));
        assert_eq!(
            terminal.ip_address_source, expected_source,
            "{name}: the terminal event must record the middleware's resolved provenance"
        );
    }
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
    let planes = build_e2e_planes();
    let app = planes.public.expect("role = all binds the public plane");

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
    let refresh_body = format!("grant_type=refresh_token&refresh_token={}", refresh_token);

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
    // Rotation is on by default: the refresh grant returns a replacement, and
    // the presented token is now retired (grace-superseded).
    let rotated_refresh_token = refresh_json["refresh_token"].as_str().unwrap();
    assert!(!rotated_refresh_token.is_empty());
    assert_ne!(rotated_refresh_token, refresh_token);

    // Step 3: POST /revoke with the *current* generation → 200
    let revoke_body = format!(
        "token={}&token_type_hint=refresh_token",
        rotated_refresh_token
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

    // Step 4: POST /token with grant_type=refresh_token (the revoked current
    // generation) → should fail as unknown: the revocation removed it.
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/token")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(format!(
                    "grant_type=refresh_token&refresh_token={rotated_refresh_token}"
                )))
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
    let planes = build_e2e_planes();
    let public = planes.public.expect("role = all binds the public plane");
    let admin = planes.admin.expect("role = all binds the admin plane");

    // Step 1: POST /internal/users on the ADMIN plane → create user
    // Use external_id "test-subject" to match the mock provider's identity claims
    let response = admin
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

    // Step 2: PUT /internal/users/{id}/claims on the ADMIN plane → {"role": "admin"}
    let response = admin
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

    // Step 3: POST /token with grant_type=authorization_code (for that user) on the
    // PUBLIC plane → get access_token. The mock provider returns
    // external_id="test-subject", matching the user we created.
    let exchange_body =
        "grant_type=authorization_code&code=test-code&redirect_uri=http://localhost/callback&provider=test";

    let response = public
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
    let payload = decode_jwt_payload(access_token);
    assert_eq!(payload["role"], "admin");
}

#[tokio::test]
async fn production_router_enforce_audit_failure_cleans_up_token_session() {
    let mut raw: RawConfig = toml::from_str(include_str!("../../../config/default.toml"))
        .expect("default test config is valid");
    raw.server.role = "exchange".to_string();
    raw.server.issuer = "https://auth.example.com".to_string();
    raw.audit.durability = "enforce".to_string();
    raw.audit.emit_threshold = "emergency".to_string();
    raw.rate_limit.enabled = false;
    let config = Config::resolve(raw).expect("enforce config resolves");

    let provider = MockIdentityProvider::new("test");
    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    providers.insert("test".to_string(), Box::new(provider));
    let sessions = MockRepository::new();
    let audit = MockAuditLog::new();
    audit.set_fail_mode(true).await;
    let service = AppService::new(
        Box::new(MockRepository::new()),
        Box::new(sessions.clone()),
        Box::new(MockKeyManager::new()),
        Box::new(audit),
        Box::new(MockUserSync::new()),
        Box::new(oidc_exchange_adapters::noop::NoopRateLimiter::new()),
        providers,
        config.clone(),
    );
    let app = build_router_with_rate_limiter(
        &config,
        service,
        Arc::new(oidc_exchange_adapters::noop::NoopRateLimiter::new()),
    );

    let response = app
        .oneshot(token_request(
            "grant_type=authorization_code&code=test-code&redirect_uri=http://localhost/callback&provider=test",
        ))
        .await
        .expect("router response");
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        sessions.get_all_sessions().await.is_empty(),
        "enforced mandatory audit failure must remove the issued session"
    );
}

#[tokio::test]
async fn production_router_enforce_revoke_audit_failure_is_status_indistinguishable() {
    let mut raw: RawConfig = toml::from_str(include_str!("../../../config/default.toml"))
        .expect("default test config is valid");
    raw.server.role = "exchange".to_string();
    raw.server.issuer = "https://auth.example.com".to_string();
    raw.audit.durability = "enforce".to_string();
    raw.rate_limit.enabled = false;
    let config = Config::resolve(raw).expect("enforce config resolves");

    let provider = MockIdentityProvider::new("test");
    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    providers.insert("test".to_string(), Box::new(provider));
    let sessions = MockRepository::new();
    let audit = MockAuditLog::new();
    let service = AppService::new(
        Box::new(MockRepository::new()),
        Box::new(sessions),
        Box::new(MockKeyManager::new()),
        Box::new(audit.clone()),
        Box::new(MockUserSync::new()),
        Box::new(oidc_exchange_adapters::noop::NoopRateLimiter::new()),
        providers,
        config.clone(),
    );
    let app = build_router_with_rate_limiter(
        &config,
        service,
        Arc::new(oidc_exchange_adapters::noop::NoopRateLimiter::new()),
    );

    let exchange = app
        .clone()
        .oneshot(token_request(
            "grant_type=authorization_code&code=test-code&redirect_uri=http://localhost/callback&provider=test",
        ))
        .await
        .expect("exchange response");
    assert_eq!(exchange.status(), StatusCode::OK);
    let refresh_token = body_to_json(exchange.into_body()).await["refresh_token"]
        .as_str()
        .expect("refresh token")
        .to_owned();
    audit.set_fail_mode(true).await;

    let existing = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/revoke")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(format!(
                    "token={refresh_token}&token_type_hint=refresh_token"
                )))
                .expect("existing revoke request"),
        )
        .await
        .expect("existing revoke response");
    let unknown = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/revoke")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(
                    "token=never-issued&token_type_hint=refresh_token",
                ))
                .expect("unknown revoke request"),
        )
        .await
        .expect("unknown revoke response");

    assert_eq!(existing.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(unknown.status(), existing.status());
    assert_eq!(
        unknown
            .headers()
            .get("content-length")
            .and_then(|value| value.to_str().ok()),
        existing
            .headers()
            .get("content-length")
            .and_then(|value| value.to_str().ok()),
        "identical RFC 7009 responses must not expose token existence"
    );
}

#[tokio::test]
async fn public_router_emits_one_mandatory_authentication_failure_at_emergency_threshold() {
    let mut raw = base_raw();
    raw.server.role = "exchange".to_string();
    raw.server.issuer = "https://auth.example.com".to_string();
    raw.audit.emit_threshold = "emergency".to_string();
    raw.rate_limit.enabled = false;
    let config = Config::resolve(raw).expect("test config resolves");

    let provider = MockIdentityProvider::new("test");
    provider.set_invalid_grant().await;
    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    providers.insert("test".to_string(), Box::new(provider));
    let audit = MockAuditLog::new();
    let service = AppService::new(
        Box::new(MockRepository::new()),
        Box::new(MockRepository::new()),
        Box::new(MockKeyManager::new()),
        Box::new(audit.clone()),
        Box::new(MockUserSync::new()),
        Box::new(oidc_exchange_adapters::noop::NoopRateLimiter::new()),
        providers,
        config.clone(),
    );
    let app = build_router_with_rate_limiter(
        &config,
        service,
        Arc::new(oidc_exchange_adapters::noop::NoopRateLimiter::new()),
    );

    let response = app
        .oneshot(token_request(
            "grant_type=authorization_code&code=bad&redirect_uri=http://localhost/callback&provider=test",
        ))
        .await
        .expect("router response");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        body_to_json(response.into_body()).await["error"],
        "invalid_grant"
    );

    let events = audit.events().await;
    assert_eq!(events.len(), 1, "core-reached failure must emit once");
    assert_eq!(events[0].event_type, AuditEventType::ValidationFailed);
}

#[tokio::test]
async fn public_router_uses_real_fixed_window_limiter_at_budget_and_after_rollover() {
    let mut raw = base_raw();
    raw.server.role = "exchange".to_string();
    raw.server.issuer = "https://auth.example.com".to_string();
    raw.rate_limit.enabled = true;
    raw.rate_limit.per_ip = 2;
    raw.rate_limit.per_ip_failures = 0;
    raw.rate_limit.per_provider = 0;
    raw.rate_limit.per_subject = 0;
    raw.rate_limit.max_entries = 16;
    let config = Config::resolve(raw).expect("test config resolves");

    let provider = MockIdentityProvider::new("test");
    let provider_view = provider.clone();
    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    providers.insert("test".to_string(), Box::new(provider));
    let service = AppService::new(
        Box::new(MockRepository::new()),
        Box::new(MockRepository::new()),
        Box::new(MockKeyManager::new()),
        Box::new(MockAuditLog::new()),
        Box::new(MockUserSync::new()),
        Box::new(oidc_exchange_adapters::noop::NoopRateLimiter::new()),
        providers,
        config.clone(),
    );
    let window = Duration::from_secs(60);
    let clock = TestClock::new(Instant::now());
    let limiter = Arc::new(
        FixedWindowRateLimiter::with_clock(
            window,
            RateLimitBudgets {
                per_ip: 2,
                per_ip_failures: 0,
                per_subject: 0,
                per_provider: 0,
            },
            16,
            Arc::new(clock.clone()),
        )
        .expect("valid fixed window limiter"),
    );
    let app = build_router_with_rate_limiter(&config, service, limiter);

    for expected in [
        StatusCode::OK,
        StatusCode::OK,
        StatusCode::TOO_MANY_REQUESTS,
    ] {
        let response = app
            .clone()
            .oneshot(token_request(
                "grant_type=authorization_code&code=test-code&redirect_uri=http://localhost/callback&provider=test",
            ))
            .await
            .expect("router response");
        assert_eq!(response.status(), expected);
    }
    assert_eq!(
        provider_view.exchange_code_call_count().await,
        2,
        "denied public requests must not invoke the provider"
    );

    clock.advance(window);
    let response = app
        .oneshot(token_request(
            "grant_type=authorization_code&code=test-code&redirect_uri=http://localhost/callback&provider=test",
        ))
        .await
        .expect("router response after window rollover");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(provider_view.exchange_code_call_count().await, 3);
}

#[tokio::test]
async fn router_denies_sixty_first_request_before_provider_work_and_sets_retry_after() {
    let mut raw = base_raw();
    raw.rate_limit.enabled = true;
    raw.rate_limit.per_ip = 60;
    raw.rate_limit.per_ip_failures = 0;
    raw.rate_limit.per_provider = 0;
    raw.rate_limit.per_subject = 0;
    raw.rate_limit.max_entries = 1024;
    raw.audit.durability = "observe".to_string();
    let config = Config::resolve(raw).expect("test config resolves");
    let provider = MockIdentityProvider::new("test");
    let audit = MockAuditLog::new();
    let public_limiter = MockRateLimiter::new();
    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    providers.insert("test".to_string(), Box::new(provider.clone()));
    let service = AppService::new(
        Box::new(MockRepository::new()),
        Box::new(MockRepository::new()),
        Box::new(MockKeyManager::new()),
        Box::new(audit.clone()),
        Box::new(MockUserSync::new()),
        Box::new(MockRateLimiter::new()),
        providers,
        config.clone(),
    );
    let app = build_router_with_rate_limiter(&config, service, Arc::new(public_limiter.clone()));
    let decisions = std::iter::once(RateLimitDecision::Deny {
        retry_after_secs: 60,
    })
    .chain(std::iter::repeat_n(RateLimitDecision::Allow, 60))
    .collect();
    public_limiter.set_decisions(decisions).await;
    let body = "grant_type=authorization_code&code=test-code&redirect_uri=http://localhost/callback&provider=test";

    for _ in 0..60 {
        let response = app.clone().oneshot(token_request(body)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
    let keys_before = public_limiter.keys().await;
    assert_eq!(keys_before.len(), 60);
    let response = app.oneshot(token_request(body)).await.unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(response.headers().get("retry-after").is_some());
    assert_eq!(provider.exchange_code_call_count().await, 60);
    let events = audit.events().await;
    let throttle_events = events
        .iter()
        .filter(|event| event.event_type == AuditEventType::ThrottleExceeded)
        .collect::<Vec<_>>();
    assert_eq!(
        throttle_events.len(),
        1,
        "direct public throttle denial emits once"
    );
    assert_eq!(throttle_events[0].ip_address.as_deref(), Some("192.0.2.1"));
}

/// S11: `/nonce` — unauthenticated and single-use-state-writing — shares the
/// server-established per-IP throttle budget with `/token`. Exhausting the
/// budget from one peer returns `429 slow_down` with `Retry-After` and emits the
/// mandatory `ThrottleExceeded`; a request with no server-established address is
/// not throttled.
#[tokio::test]
async fn nonce_shares_the_public_per_ip_throttle_budget() {
    fn nonce_request(client_addr: oidc_exchange_core::domain::ClientAddr) -> Request<Body> {
        let mut request = Request::builder()
            .method("POST")
            .uri("/nonce")
            .body(Body::empty())
            .unwrap();
        request
            .extensions_mut()
            .insert(oidc_exchange::middleware::audit_context::AuditContext {
                client_addr,
                user_agent: None,
                device_id: None,
            });
        request
    }
    let peer = || oidc_exchange_core::domain::ClientAddr::Peer("192.0.2.1".parse().unwrap());

    let mut raw = base_raw();
    raw.grants.id_token = true;
    raw.rate_limit.enabled = true;
    raw.rate_limit.per_ip = 2;
    raw.rate_limit.per_ip_failures = 0;
    raw.rate_limit.per_provider = 0;
    raw.rate_limit.per_subject = 0;
    raw.rate_limit.max_entries = 1024;
    raw.audit.durability = "observe".to_string();
    raw.server.role = "exchange".to_string();
    raw.server.issuer = "https://auth.example.com".to_string();
    let config = Config::resolve(raw).expect("test config resolves");

    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    providers.insert(
        "test".to_string(),
        Box::new(MockIdentityProvider::new("test")),
    );
    let audit = MockAuditLog::new();
    let public_limiter = MockRateLimiter::new();
    let service = AppService::new(
        Box::new(MockRepository::new()),
        Box::new(MockRepository::new()),
        Box::new(MockKeyManager::new()),
        Box::new(audit.clone()),
        Box::new(MockUserSync::new()),
        Box::new(MockRateLimiter::new()),
        providers,
        config.clone(),
    );
    let app = build_router_with_rate_limiter(&config, service, Arc::new(public_limiter.clone()));
    // Two allowed nonce mints, then a denial — one decision consumed per
    // request. Decisions are consumed back-to-front (`Vec::pop`), so the denial
    // is listed first to arrive last.
    public_limiter
        .set_decisions(vec![
            RateLimitDecision::Deny {
                retry_after_secs: 60,
            },
            RateLimitDecision::Allow,
            RateLimitDecision::Allow,
        ])
        .await;

    for _ in 0..2 {
        let response = app.clone().oneshot(nonce_request(peer())).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "under-budget nonce mints"
        );
    }

    let response = app.clone().oneshot(nonce_request(peer())).await.unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .expect("a throttled nonce must carry a numeric Retry-After");
    assert!(retry_after >= 1, "Retry-After must be at least one second");
    assert_eq!(
        body_to_json(response.into_body()).await["error"],
        "slow_down"
    );

    // The budget is keyed exactly as `/token`'s is — the shared per-IP key.
    let keys = public_limiter.keys().await;
    assert!(
        keys.iter()
            .all(|key| *key == RateLimitKey::ClientAddr("192.0.2.1".parse().unwrap())),
        "/nonce must consume the shared per-IP ClientAddr budget: {keys:?}"
    );

    // The denial emitted exactly one mandatory ThrottleExceeded.
    let throttle_events = audit
        .events()
        .await
        .into_iter()
        .filter(|event| event.event_type == AuditEventType::ThrottleExceeded)
        .collect::<Vec<_>>();
    assert_eq!(
        throttle_events.len(),
        1,
        "nonce denial emits ThrottleExceeded once"
    );

    // A request with no server-established address is never throttled and
    // consumes no budget (early return before check_and_consume).
    let keys_before = public_limiter.keys().await.len();
    let response = app
        .oneshot(nonce_request(
            oidc_exchange_core::domain::ClientAddr::Unknown,
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "a nonce request with no server-established address must not be throttled"
    );
    assert_eq!(
        public_limiter.keys().await.len(),
        keys_before,
        "an unthrottled nonce request must consume no per-IP budget"
    );
}

#[tokio::test]
async fn router_counts_invalid_grant_failures_but_not_malformed_requests() {
    let mut raw = base_raw();
    raw.rate_limit.enabled = true;
    raw.rate_limit.per_ip = 0;
    raw.rate_limit.per_ip_failures = 1;
    raw.rate_limit.per_provider = 0;
    raw.rate_limit.per_subject = 0;
    raw.audit.durability = "observe".to_string();
    let provider = MockIdentityProvider::new("test");
    provider.set_invalid_grant().await;
    let public_limiter = MockRateLimiter::new();
    let app = build_throttled_router(
        raw,
        provider,
        MockRateLimiter::new(),
        public_limiter.clone(),
    );
    let invalid_grant = "grant_type=authorization_code&code=bad&redirect_uri=http://localhost/callback&provider=test";

    let response = app
        .clone()
        .oneshot(token_request(invalid_grant))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        body_to_json(response.into_body()).await["error"],
        "invalid_grant"
    );
    let response = app
        .oneshot(token_request("grant_type=authorization_code"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let keys = public_limiter.keys().await;
    assert_eq!(
        keys.iter()
            .filter(|key| matches!(
                key,
                oidc_exchange_core::domain::RateLimitKey::ClientAddrFailure(_)
            ))
            .count(),
        1,
        "invalid_grant consumes exactly one failure-IP budget while malformed input does not"
    );
}

#[tokio::test]
async fn e2e_registration_policy_existing_users_only() {
    let planes = build_e2e_planes_with_config(test_config("existing_users_only"));
    let public = planes.public.expect("role = all binds the public plane");
    let admin = planes.admin.expect("role = all binds the admin plane");

    // Step 1: POST /token on the PUBLIC plane → 403 access_denied (user doesn't exist)
    let exchange_body =
        "grant_type=authorization_code&code=test-code&redirect_uri=http://localhost/callback&provider=test";

    let response = public
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

    // Step 2: POST /internal/users on the ADMIN plane → create user with matching external_id
    let response = admin
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

    // Step 3: POST /token on the PUBLIC plane → 200 success (user now exists)
    let response = public
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
