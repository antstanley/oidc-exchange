//! E2E coverage for the task-05 operator principal and attribution: every
//! request below runs through the production admin router
//! (`bootstrap::build_routers`, full middleware stack) so the tests exercise
//! exactly what a role = "admin" process serves.
//!
//! Properties under test:
//! - each mechanism (`shared_secret`, `operator_token`, `mtls`) authenticates
//!   into its documented principal, and malformed credentials are rejected;
//! - failed attempts draw one unit from the `OperatorAuth` budget and emit
//!   one mandatory-channel security event with a fixed reason; successes draw
//!   nothing; a lockout short-circuits to `429` before any credential is
//!   evaluated;
//! - successful mutations carry the authenticated principal on their audit
//!   events (explicitly unattributed under the shared secret), while
//!   exchange-plane events keep null operator attribution.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use http_body_util::BodyExt;
use serde_json::json;
use tower::ServiceExt;

use oidc_exchange::bootstrap::build_routers;
use oidc_exchange_adapters::local_keys::LocalKeyManager;
use oidc_exchange_core::config::{AppConfig, LocalKeyConfig};
use oidc_exchange_core::domain::{AuditEvent, AuditEventType, AuditOutcome};
use oidc_exchange_core::ports::{IdentityProvider, KeyManager};
use oidc_exchange_core::service::AppService;
use oidc_exchange_test_utils::{
    MockAuditLog, MockIdentityProvider, MockKeyManager, MockRateLimiter, MockRepository,
    MockUserSync,
};

/// The fixed audit classification of an `OperatorAuthenticationFailed`
/// security event.
const UNAUTHORIZED_AUDIT_TYPE: &str = "unauthorized";
/// The fixed audit classification of a lockout (`ThrottleExceeded`).
const THROTTLE_EXCEEDED_AUDIT_TYPE: &str = "throttle_exceeded";

/// Render an outcome's fixed failure reason, if it is a failure at all.
fn outcome_reason(outcome: &AuditOutcome) -> Option<&str> {
    match outcome {
        AuditOutcome::Failure { reason } => Some(reason.as_str()),
        AuditOutcome::Success => None,
    }
}

/// The canonical wire spelling of an audit event's type.
fn event_type_str(event: &AuditEvent) -> &'static str {
    match event.event_type {
        AuditEventType::Unauthorized => "unauthorized",
        AuditEventType::ThrottleExceeded => "throttle_exceeded",
        _ => "other",
    }
}

const TEST_SECRET: &str = "operator-auth-e2e-shared-secret-value";
const MTLS_SUBJECT_HEADER: &str = "x-client-cert-subject";
const CERT_SUBJECT: &str = "CN=ops.example.com,O=Example";
const OPERATOR_SUBJECT: &str = "usr_operator_alice";
/// A deterministic PKCS#8 Ed25519 key used only by this suite: it signs the
/// operator tokens the token-mechanism tests mint and verify. Test material,
/// not a secret.
const TEST_SIGNING_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIKi3xyKlMTIijQsU1128JaI+z+S0aRZBaLKJmWijGFPw
-----END PRIVATE KEY-----
";
const ISSUER: &str = "https://auth.example.com";

/// Everything a test needs to drive and observe the admin plane: the router,
/// plus handles back into the mocks the service was built over.
struct AdminPlane {
    app: axum::Router,
    audit: MockAuditLog,
    limiter: MockRateLimiter,
}

impl AdminPlane {
    /// Send a request to the plane with the given bearer credential and peer
    /// address. The peer is inserted as `ConnectInfo` — the same extension
    /// the real listener provides — because the throttle keys off it.
    async fn send(
        &self,
        method: &str,
        uri: &str,
        bearer: Option<&str>,
        body: Option<String>,
    ) -> axum::response::Response {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("host", "admin.test")
            // Always JSON-typed: mutating handlers extract Json bodies, and a
            // missing content-type would 415 before auth even matters.
            .header("content-type", "application/json");
        if let Some(token) = bearer {
            builder = builder.header("authorization", format!("Bearer {token}"));
        }
        let mut request = builder
            .body(Body::from(body.unwrap_or_default()))
            .expect("test requests always build");
        request.extensions_mut().insert(ConnectInfo(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(192, 0, 2, 7)),
            40000,
        )));
        self.app
            .clone()
            .oneshot(request)
            .await
            .expect("the admin router always answers")
    }
}

fn mock_service(
    config: &AppConfig,
    providers: HashMap<String, Box<dyn IdentityProvider>>,
) -> (AppService, MockAuditLog, MockRateLimiter) {
    let audit = MockAuditLog::new();
    let limiter = MockRateLimiter::new();
    let service = AppService::new(
        Box::new(MockRepository::new()),
        Box::new(MockRepository::new()),
        Box::new(MockKeyManager::new()),
        Box::new(audit.clone()),
        Box::new(MockUserSync::new()),
        Box::new(limiter.clone()),
        providers,
        config.clone(),
    );
    (service, audit, limiter)
}

fn admin_config(auth_methods: &[&str]) -> AppConfig {
    let mut config = AppConfig::default();
    config.server.role = "admin".to_string();
    config.server.issuer = ISSUER.to_string();
    config.internal_api.enabled = true;
    config.internal_api.auth_methods = auth_methods.iter().map(|s| s.to_string()).collect();
    config.internal_api.shared_secret = Some(TEST_SECRET.to_string());
    config
}

async fn body_to_json(body: Body) -> serde_json::Value {
    let bytes = body.collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

// ---------------------------------------------------------------------------
// Shared-secret mechanism: rejection reasons, budget accounting, attribution
// ---------------------------------------------------------------------------

#[tokio::test]
async fn missing_credential_is_rejected_and_audited_and_consumes_budget() {
    let config = admin_config(&["shared_secret"]);
    let (service, audit, limiter) = mock_service(&config, HashMap::new());
    let routers = build_routers(&config, service).expect("the admin config builds routers");
    let plane = AdminPlane {
        app: routers.admin.expect("role admin binds the admin plane"),
        audit,
        limiter,
    };

    let response = plane.send("GET", "/internal/stats", None, None).await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["error"], "unauthorized");
    assert_eq!(json["error_description"], "authentication required");

    // Exactly one failure was recorded on the budget...
    assert_eq!(plane.limiter.consume_calls().await, 1);
    assert_eq!(plane.limiter.check_calls().await, 1);

    // ...and exactly one security event carries the fixed reason. The
    // presented credential (there was none) never appears anywhere.
    let events = plane.audit.events().await;
    assert_eq!(events.len(), 1, "one attempt, one event: {events:?}");
    assert_eq!(event_type_str(&events[0]), UNAUTHORIZED_AUDIT_TYPE);
    assert_eq!(
        outcome_reason(&events[0].outcome),
        Some("missing_credential")
    );
    assert!(events[0].operator.is_none());
}

#[tokio::test]
async fn wrong_secret_is_rejected_as_invalid_credential() {
    let config = admin_config(&["shared_secret"]);
    let (service, audit, limiter) = mock_service(&config, HashMap::new());
    let routers = build_routers(&config, service).expect("the admin config builds routers");
    let plane = AdminPlane {
        app: routers.admin.expect("role admin binds the admin plane"),
        audit,
        limiter,
    };

    let response = plane
        .send("GET", "/internal/stats", Some("not-the-secret"), None)
        .await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["error_description"], "invalid credential");

    assert_eq!(plane.limiter.consume_calls().await, 1);
    let events = plane.audit.events().await;
    assert_eq!(events.len(), 1);
    assert_eq!(
        outcome_reason(&events[0].outcome),
        Some("invalid_credential")
    );
    // The rejected guess itself must not be recorded anywhere in the event.
    let rendered = format!("{:?}", events[0]);
    assert!(!rendered.contains("not-the-secret"));
}

#[tokio::test]
async fn shared_secret_success_creates_a_user_attributed_unattributed_without_consuming_budget() {
    let config = admin_config(&["shared_secret"]);
    let (service, audit, limiter) = mock_service(&config, HashMap::new());
    let routers = build_routers(&config, service).expect("the admin config builds routers");
    let plane = AdminPlane {
        app: routers.admin.expect("role admin binds the admin plane"),
        audit,
        limiter,
    };

    let response = plane
        .send(
            "POST",
            "/internal/users",
            Some(TEST_SECRET),
            Some(json!({"external_id": "ext-1", "provider": "google"}).to_string()),
        )
        .await;

    assert_eq!(response.status(), StatusCode::CREATED);
    let user = body_to_json(response.into_body()).await;
    let user_id = user["id"].as_str().unwrap().to_string();
    assert!(user_id.starts_with("usr_"));

    // A working credential draws nothing down: only failures consume.
    assert_eq!(plane.limiter.consume_calls().await, 0);

    // The mutation's audit event names the actor (the user acted upon) AND
    // the operator (who performed it), with the shared secret's reserved
    // unattributed shape rather than an omitted identity.
    let events = plane.audit.events().await;
    assert_eq!(events.len(), 1, "exactly the mutation event: {events:?}");
    assert_eq!(events[0].actor.as_deref(), Some(user_id.as_str()));
    let operator = events[0]
        .operator
        .as_ref()
        .expect("mutations are attributed");
    assert_eq!(operator.id, "unattributed");
    match operator.mechanism {
        oidc_exchange_core::domain::OperatorAuthMechanism::SharedSecret => {}
        other => panic!("shared secret must attribute as shared_secret, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Throttle: lockout short-circuits before any credential evaluation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn locked_out_peers_get_429_with_retry_after_before_credentials_are_evaluated() {
    let config = admin_config(&["shared_secret"]);
    let (service, audit, limiter) = mock_service(&config, HashMap::new());
    let routers = build_routers(&config, service).expect("the admin config builds routers");
    let plane = AdminPlane {
        app: routers.admin.expect("role admin binds the admin plane"),
        audit,
        limiter,
    };
    plane.limiter.set_deny_mode(true).await;

    // Even a VALID credential is refused while locked out: the consultation
    // precedes any credential evaluation.
    let response = plane
        .send("GET", "/internal/stats", Some(TEST_SECRET), None)
        .await;

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);

    // The lockout response advertises the remaining seconds as a number.
    let retry_after = response
        .headers()
        .get("Retry-After")
        .expect("a lockout response advertises Retry-After")
        .to_str()
        .expect("retry-after is numeric")
        .to_string();
    assert_eq!(retry_after.parse::<u64>().unwrap(), 60);

    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["error"], "slow_down");

    // The denial consumed nothing: check ran, consume did not (a Deny
    // short-circuit records no further failure).
    assert_eq!(plane.limiter.check_calls().await, 1);
    assert_eq!(plane.limiter.consume_calls().await, 0);

    // The lockout is audited as ThrottleExceeded, distinct from auth failures.
    let events = plane.audit.events().await;
    assert_eq!(events.len(), 1);
    assert_eq!(event_type_str(&events[0]), THROTTLE_EXCEEDED_AUDIT_TYPE);
    assert!(events[0].operator.is_none());
}

// ---------------------------------------------------------------------------
// mTLS mechanism: proxy-asserted subject becomes the named principal
// ---------------------------------------------------------------------------

#[tokio::test]
async fn mtls_header_authenticates_the_certificate_subject() {
    let mut config = admin_config(&["mtls"]);
    config.internal_api.shared_secret = None;
    let (service, audit, _limiter) = mock_service(&config, HashMap::new());
    let routers = build_routers(&config, service).expect("the mtls config builds routers");
    let plane_app = routers.admin.expect("role admin binds the admin plane");

    let builder = Request::builder()
        .method("POST")
        .uri("/internal/users")
        .header(MTLS_SUBJECT_HEADER, CERT_SUBJECT)
        .header("content-type", "application/json");
    let mut request = builder
        .body(Body::from(
            json!({"external_id": "ext-mtls", "provider": "google"}).to_string(),
        ))
        .unwrap();
    request.extensions_mut().insert(ConnectInfo(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(192, 0, 2, 8)),
        40001,
    )));
    let response = plane_app.clone().oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let events = audit.events().await;
    assert_eq!(events.len(), 1);
    let operator = events[0]
        .operator
        .as_ref()
        .expect("mutations are attributed");
    assert_eq!(operator.id, CERT_SUBJECT);
    match operator.mechanism {
        oidc_exchange_core::domain::OperatorAuthMechanism::MutualTls => {}
        other => panic!("mtls must attribute as mtls, got {other:?}"),
    }

    // Negative space: without the header there is no credential at all.
    let mut request = Request::builder()
        .method("GET")
        .uri("/internal/stats")
        .body(Body::empty())
        .unwrap();
    request.extensions_mut().insert(ConnectInfo(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(192, 0, 2, 9)),
        40002,
    )));
    let response = plane_app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// The mtls header is trusted only where the mechanism is mounted. On the
/// public plane no internal route exists to consult it — presenting the very
/// same assertion against the public router changes nothing.
#[tokio::test]
async fn mtls_headers_change_nothing_on_the_public_plane() {
    let mut config = admin_config(&["mtls"]);
    config.internal_api.shared_secret = None;
    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    providers.insert(
        "test".to_string(),
        Box::new(MockIdentityProvider::new("test")),
    );
    let (service, _audit, _limiter) = mock_service(&config, providers);

    let mut config_all = config.clone();
    config_all.server.role = "all".to_string();
    let routers =
        build_routers(&config_all, service).expect("the all-role mtls config builds routers");
    let public = routers.public.expect("role all binds the public plane");

    for uri in ["/internal/stats", "/internal/users"] {
        let mut request = Request::builder()
            .method("GET")
            .uri(uri)
            .header(MTLS_SUBJECT_HEADER, CERT_SUBJECT)
            .body(Body::empty())
            .unwrap();
        request.extensions_mut().insert(ConnectInfo(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10)),
            40003,
        )));
        let response = public.clone().oneshot(request).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "{uri} must be absent from the public plane even with an asserted subject"
        );
    }

    // And the public exchange surface still works normally with the header
    // present: it is simply never consulted there.
    let mut request = Request::builder()
        .method("GET")
        .uri("/health")
        .header(MTLS_SUBJECT_HEADER, CERT_SUBJECT)
        .body(Body::empty())
        .unwrap();
    request.extensions_mut().insert(ConnectInfo(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10)),
        40004,
    )));
    let response = public.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

// ---------------------------------------------------------------------------
// Operator-token mechanism: real key manager, verified claims, named subject
// ---------------------------------------------------------------------------

fn local_key_config(dir: &std::path::Path) -> LocalKeyConfig {
    let key_path = dir.join("operator-signing-key.pem");
    std::fs::write(&key_path, TEST_SIGNING_KEY_PEM).expect("writing the test key succeeds");
    LocalKeyConfig {
        private_key_path: key_path.to_string_lossy().to_string(),
        algorithm: "EdDSA".to_string(),
        kid: "operator-e2e-key".to_string(),
    }
}

fn b64(data: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(data)
}

/// Mint an operator token signed by the same local key the service verifies
/// against, with per-test claim overrides applied.
async fn mint_operator_token(
    keys: &LocalKeyManager,
    claims_overrides: impl FnOnce(&mut serde_json::Value),
    now: u64,
) -> String {
    let mut payload = json!({
        "iss": ISSUER,
        "aud": "internal",
        "sub": OPERATOR_SUBJECT,
        "exp": now + 600,
        "iat": now,
        "role": "admin",
    });
    claims_overrides(&mut payload);

    let header = json!({"alg": keys.algorithm(), "typ": "JWT"});
    let signing_input = format!(
        "{}.{}",
        b64(serde_json::to_vec(&header).unwrap().as_slice()),
        b64(serde_json::to_vec(&payload).unwrap().as_slice()),
    );
    let signature = keys.sign(signing_input.as_bytes()).await.unwrap();
    format!("{signing_input}.{}", b64(&signature))
}

#[tokio::test]
async fn valid_operator_token_authenticates_to_its_subject() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut config = admin_config(&["operator_token"]);
    config.key_manager.adapter = "local".to_string();
    config.key_manager.local = Some(local_key_config(dir.path()));

    let signing_keys =
        LocalKeyManager::from_pem(TEST_SIGNING_KEY_PEM.as_bytes(), "EdDSA", "operator-e2e-key")
            .expect("the embedded test key parses");

    let (service, audit, _limiter) = mock_service(&config, HashMap::new());
    let routers = build_routers(&config, service).expect("the operator_token config builds");
    let plane_app = routers.admin.expect("role admin binds the admin plane");

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let token = mint_operator_token(&signing_keys, |_| {}, now).await;

    let mut request = Request::builder()
        .method("POST")
        .uri("/internal/users")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"external_id": "ext-tok", "provider": "google"}).to_string(),
        ))
        .unwrap();
    request.extensions_mut().insert(ConnectInfo(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(192, 0, 2, 11)),
        40005,
    )));
    let response = plane_app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let events = audit.events().await;
    assert_eq!(events.len(), 1);
    let operator = events[0]
        .operator
        .as_ref()
        .expect("mutations are attributed");
    assert_eq!(operator.id, OPERATOR_SUBJECT);
    match operator.mechanism {
        oidc_exchange_core::domain::OperatorAuthMechanism::OperatorToken => {}
        other => panic!("operator tokens must attribute as operator_token, got {other:?}"),
    }
}

#[tokio::test]
async fn defective_operator_tokens_are_rejected_with_invalid_credential() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut config = admin_config(&["operator_token"]);
    config.key_manager.adapter = "local".to_string();
    config.key_manager.local = Some(local_key_config(dir.path()));

    let signing_keys =
        LocalKeyManager::from_pem(TEST_SIGNING_KEY_PEM.as_bytes(), "EdDSA", "operator-e2e-key")
            .expect("the embedded test key parses");

    let (service, audit, _limiter) = mock_service(&config, HashMap::new());
    let routers = build_routers(&config, service).expect("the operator_token config builds");
    let plane_app = routers.admin.expect("role admin binds the admin plane");

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    // Expired well beyond the clock-skew leeway.
    let expired = mint_operator_token(
        &signing_keys,
        |claims| claims["exp"] = json!(now - 3600),
        now,
    )
    .await;
    // Wrong audience.
    let wrong_aud = mint_operator_token(
        &signing_keys,
        |claims| claims["aud"] = json!("https://api.example.com"),
        now,
    )
    .await;
    // Missing the required claim value.
    let missing_role = mint_operator_token(
        &signing_keys,
        |claims| {
            claims.as_object_mut().unwrap().remove("role");
        },
        now,
    )
    .await;
    // Wrong issuer.
    let wrong_iss = mint_operator_token(
        &signing_keys,
        |claims| claims["iss"] = json!("https://evil.example.com"),
        now,
    )
    .await;

    for (label, token) in [
        ("expired", expired),
        ("wrong audience", wrong_aud),
        ("missing required claim", missing_role),
        ("wrong issuer", wrong_iss),
    ] {
        let mut request = Request::builder()
            .method("GET")
            .uri("/internal/stats")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        request.extensions_mut().insert(ConnectInfo(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(192, 0, 2, 12)),
            40006,
        )));
        let response = plane_app.clone().oneshot(request).await.unwrap();

        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "{label} must be rejected"
        );
    }

    // Every rejection drew one unit and emitted one fixed-reason event; the
    // tokens themselves appear nowhere in the audit stream.
    let events = audit.events().await;
    assert_eq!(events.len(), 4, "one event per rejected attempt");
    for event in &events {
        assert_eq!(outcome_reason(&event.outcome), Some("invalid_credential"));
    }
    let rendered = format!("{events:?}");
    assert!(
        !rendered.contains("eyJ"),
        "no raw JWT may reach the audit stream"
    );
}

// ---------------------------------------------------------------------------
// Exchange plane: events keep null operator attribution
// ---------------------------------------------------------------------------

#[tokio::test]
async fn exchange_plane_events_retain_null_operator_attribution() {
    let mut config = admin_config(&["shared_secret"]);
    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    providers.insert(
        "test".to_string(),
        Box::new(MockIdentityProvider::new("test")),
    );
    let (service, audit, _limiter) = mock_service(&config, providers);

    config.server.role = "all".to_string();
    let routers = build_routers(&config, service).expect("the all-role config builds");
    let public = routers.public.expect("role all binds the public plane");

    let exchange_body =
        "grant_type=authorization_code&code=test-code&redirect_uri=http://localhost/callback&provider=test";
    let mut request = Request::builder()
        .method("POST")
        .uri("/token")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(exchange_body))
        .unwrap();
    request.extensions_mut().insert(ConnectInfo(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(192, 0, 2, 13)),
        40007,
    )));
    let response = public.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    // Every event the exchange flow emitted carries no operator: attribution
    // exists only on /internal/* operations.
    let events: Vec<AuditEvent> = audit.events().await;
    assert!(
        !audit.events().await.is_empty(),
        "the exchange flow emits at least one auditable event"
    );
    for event in &events {
        assert!(
            event.operator.is_none(),
            "exchange-plane events must stay unattributed: {:?}",
            event.event_type
        );
    }
}
