//! End-to-end HTTP leak-oracle suite (plan task 07): the public error-oracle checks
//! consolidated onto a real axum router carrying the production middleware stack.
//! (The request-ID boundary and silent-rejection assertions live in
//! `http_leak_regression.rs`; this file's oracle tests assert the request-id side of
//! the same control — a client-chosen id is echoed even on failures, and the operator
//! diagnostic for that failure fires under the span carrying it.)
//!
//! Everything here drives `POST /token` through the same layers bootstrap installs —
//! request-id outermost, audit-context inside it — under a capturing subscriber that
//! records every rendered telemetry fragment *and* the request span each event fired
//! in:
//!
//! - an unknown `kid` is never echoed to the caller but stays in the operator's warn
//!   diagnostic, which fires under the request span so it carries the echoed id;
//! - distinct validation failures are indistinguishable in bodies;
//! - a hostile upstream echoing submitted credentials back produces a generic 502 body
//!   and an error log whose only credential-shaped content is `[REDACTED]`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::middleware::from_fn;
use axum::response::Response;
use axum::Router;
use http_body_util::BodyExt;
use tower::ServiceExt;
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry::LookupSpan;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use oidc_exchange::middleware::audit_context::audit_context_layer;
use oidc_exchange::middleware::request_id::request_id_layer;
use oidc_exchange::routes::public_routes;
use oidc_exchange::state::AppState;
use oidc_exchange_core::config::AppConfig;
use oidc_exchange_core::domain::{IdentityClaims, ProviderTokens};
use oidc_exchange_core::error::{Error, Result};
use oidc_exchange_core::ports::IdentityProvider;
use oidc_exchange_core::service::AppService;
use oidc_exchange_test_utils::{MockAuditLog, MockKeyManager, MockRepository, MockUserSync};

// ---------------------------------------------------------------------------
// Capture harness: every rendered fragment + each event's enclosing request id
// ---------------------------------------------------------------------------

/// Fields declared on a span, captured at creation so events can be correlated back
/// to the per-request span.
#[derive(Default)]
struct SpanFields(HashMap<String, String>);

struct FieldVisitor<'a>(&'a mut HashMap<String, String>);

impl tracing::field::Visit for FieldVisitor<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0
            .insert(field.name().to_string(), format!("{value:?}"));
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
}

#[derive(Clone, Default)]
struct TelemetryCapture {
    fragments: Arc<Mutex<Vec<String>>>,
    event_request_ids: Arc<Mutex<Vec<String>>>,
}

struct FragmentVisitor {
    capture: TelemetryCapture,
}

impl tracing::field::Visit for FragmentVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.capture
            .fragments
            .lock()
            .expect("capture mutex must not be poisoned")
            .push(format!("{}={}", field.name(), value));
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.capture
            .fragments
            .lock()
            .expect("capture mutex must not be poisoned")
            .push(format!("{}={:?}", field.name(), value));
    }
}

impl<S> Layer<S> for TelemetryCapture
where
    S: tracing::Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        id: &tracing::span::Id,
        ctx: Context<'_, S>,
    ) {
        self.fragments
            .lock()
            .expect("capture mutex must not be poisoned")
            .push(format!("span:{}", attrs.metadata().name()));
        attrs.record(&mut FragmentVisitor {
            capture: self.clone(),
        });
        // Stash the declared fields on the span so events can later be correlated to
        // the per-request span carrying `request_id`.
        let mut fields = SpanFields::default();
        attrs.record(&mut FieldVisitor(&mut fields.0));
        let span = ctx
            .span(id)
            .expect("span must exist immediately after creation");
        span.extensions_mut().insert(fields);
    }

    fn on_record(
        &self,
        id: &tracing::span::Id,
        values: &tracing::span::Record<'_>,
        ctx: Context<'_, S>,
    ) {
        {
            let span = ctx
                .span(id)
                .expect("span must exist when recording new values");
            let mut extensions = span.extensions_mut();
            if let Some(fields) = extensions.get_mut::<SpanFields>() {
                values.record(&mut FieldVisitor(&mut fields.0));
            }
        }
        values.record(&mut FragmentVisitor {
            capture: self.clone(),
        });
    }

    fn on_event(&self, event: &tracing::Event<'_>, ctx: Context<'_, S>) {
        self.fragments
            .lock()
            .expect("capture mutex must not be poisoned")
            .push(format!("event:{}", event.metadata().name()));
        event.record(&mut FragmentVisitor {
            capture: self.clone(),
        });

        // Correlation check: walk the event's span scope and record the innermost
        // request_id found, so tests can assert operator diagnostics carry the same id
        // the client saw echoed. `from_root()` yields outermost first, so the last
        // span carrying the field is the innermost one.
        if let Some(scope) = ctx.event_scope(event) {
            let mut innermost: Option<String> = None;
            for span in scope.from_root() {
                let extensions = span.extensions();
                if let Some(fields) = extensions.get::<SpanFields>() {
                    if let Some(request_id) = fields.0.get("request_id") {
                        innermost = Some(request_id.clone());
                    }
                }
            }
            if let Some(request_id) = innermost {
                self.event_request_ids
                    .lock()
                    .expect("capture mutex must not be poisoned")
                    .push(request_id);
            }
        }
    }
}

fn install_capture() -> (
    tracing::subscriber::DefaultGuard,
    Arc<Mutex<Vec<String>>>,
    Arc<Mutex<Vec<String>>>,
) {
    let capture = TelemetryCapture::default();
    let fragments = capture.fragments.clone();
    let event_request_ids = capture.event_request_ids.clone();
    let guard = tracing::subscriber::set_default(tracing_subscriber::registry().with(capture));
    (guard, fragments, event_request_ids)
}

/// Percent-decode `%XX` escapes so assertions also cover encoded echo shapes. Mirrors
/// the production decoder the upstream redactor uses.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if let (Some(&hi), Some(&lo)) = (bytes.get(i + 1), bytes.get(i + 2)) {
                if let (Some(h), Some(l)) = (
                    (hi as char).to_digit(16).map(|v| v as u8),
                    (lo as char).to_digit(16).map(|v| v as u8),
                ) {
                    out.push((h << 4) | l);
                    i += 3;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Neither the literal sentinel nor its percent-encoded form may appear in any
/// rendered fragment.
fn assert_absent_plain_and_encoded(fragments: &[String], sentinel: &str) {
    let joined = fragments.join("\n");
    assert!(
        !joined.contains(sentinel),
        "sentinel {sentinel:?} must never reach telemetry, got: {joined}"
    );
    let decoded = percent_decode(&joined);
    assert!(
        !decoded.contains(sentinel),
        "percent-decoded telemetry must not contain {sentinel:?}, got: {decoded}"
    );
}

// ---------------------------------------------------------------------------
// App construction — production middleware stack over mock adapters
// ---------------------------------------------------------------------------

/// An IdentityProvider whose ID-token validation always fails with a fixed reason —
/// the injection point for the unknown-kid / bad-signature / expired /
/// wrong-audience oracles without a real JWKS.
struct FailingValidationProvider {
    reason: String,
}

#[async_trait]
impl IdentityProvider for FailingValidationProvider {
    async fn exchange_code(&self, _code: &str, _redirect_uri: &str) -> Result<ProviderTokens> {
        Ok(ProviderTokens {
            id_token: "corpus-failing-id-token".to_string(),
            refresh_token: None,
            access_token: None,
        })
    }

    async fn validate_id_token(&self, _id_token: &str) -> Result<IdentityClaims> {
        Err(Error::InvalidGrant {
            reason: self.reason.clone(),
        })
    }

    async fn revoke_token(&self, _token: &str) -> Result<()> {
        Ok(())
    }

    fn provider_id(&self) -> &str {
        "failing"
    }
}

/// Router with the production middleware order (request-id outermost, audit-context
/// inside it) over mock backends and the given providers map.
fn build_app(providers: HashMap<String, Box<dyn IdentityProvider>>) -> Router {
    let mut config = AppConfig::default();
    config.server.issuer = "https://auth.example.com".to_string();

    let session_repo = MockRepository::new();
    let service = AppService::new(
        Box::new(MockRepository::new()),
        Box::new(session_repo),
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
        .layer(from_fn(audit_context_layer))
        .layer(from_fn(request_id_layer))
        .with_state(state)
}

async fn post_form(app: Router, path: &str, body: String, request_id: Option<&str>) -> Response {
    let mut builder = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/x-www-form-urlencoded");
    if let Some(id) = request_id {
        builder = builder.header("x-request-id", id);
    }
    app.oneshot(builder.body(Body::from(body)).unwrap())
        .await
        .unwrap()
}

async fn body_to_json(body: Body) -> serde_json::Value {
    let bytes = body.collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}
// ---------------------------------------------------------------------------
// 2. Public error oracle over HTTP
// ---------------------------------------------------------------------------

#[tokio::test]
async fn unknown_kid_is_not_echoed_but_stays_in_the_operators_warn_log() {
    const KID_SENTINEL: &str = "SENTINEL-UNKNOWN-KID-VALUE";
    let (_guard, fragments, event_ids) = install_capture();

    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    providers.insert(
        "failing".to_string(),
        Box::new(FailingValidationProvider {
            reason: format!("No matching key for kid: {KID_SENTINEL} (after forced refetch)"),
        }),
    );
    let app = build_app(providers);

    let response = post_form(
        app,
        "/token",
        "grant_type=authorization_code&code=c\
         &redirect_uri=https://client.example.com/callback&provider=failing"
            .to_string(),
        Some("oracle-correlation-id"),
    )
    .await;

    // Caller side: generic class, static description, no kid, id still echoed.
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response
            .headers()
            .get("x-request-id")
            .expect("response must carry x-request-id")
            .to_str()
            .expect("echoed id must be visible ASCII"),
        "oracle-correlation-id",
        "the client-chosen id must be echoed even on failures"
    );
    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["error"], "invalid_grant");
    let description = json["error_description"].as_str().unwrap();
    assert_eq!(
        description, "the provided grant could not be validated",
        "the body must carry exactly the static description"
    );
    assert!(
        !description.contains(KID_SENTINEL),
        "an unknown kid must never be echoed to the caller"
    );

    // Operator side: the mapped diagnostic retains the kid AND fires under the request
    // span, correlating the generic body with the full internal reason.
    let captured = fragments
        .lock()
        .expect("mutex must not be poisoned")
        .clone();
    assert!(
        captured.iter().any(|f| f.contains(KID_SENTINEL)),
        "the operator diagnostic must retain the kid, got {captured:?}"
    );
    assert!(
        captured
            .iter()
            .any(|f| f.contains("client fault mapped to error response")),
        "the 4xx mapping must have emitted its client-fault warn event"
    );
    let ids = event_ids
        .lock()
        .expect("mutex must not be poisoned")
        .clone();
    assert!(
        ids.iter().any(|id| id == "oracle-correlation-id"),
        "the mapped error's diagnostic must fire under the request span, got {ids:?}"
    );
}

/// Distinct validation failures are indistinguishable to the caller over HTTP: same
/// status, same code, same static description — regardless of which internal step
/// rejected the grant.
#[tokio::test]
async fn validation_failure_classes_are_indistinguishable_over_http() {
    let reasons = [
        "JWT validation failed: InvalidSignature",
        "JWT validation failed: ExpiredSignature",
        "JWT validation failed: InvalidAudience",
    ];

    let mut bodies = Vec::new();
    for reason in reasons {
        let (_guard, fragments, _ids) = install_capture();
        let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
        providers.insert(
            "failing".to_string(),
            Box::new(FailingValidationProvider {
                reason: reason.to_string(),
            }),
        );
        let app = build_app(providers);

        let response = post_form(
            app,
            "/token",
            "grant_type=authorization_code&code=c\
             &redirect_uri=https://client.example.com/callback&provider=failing"
                .to_string(),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = body_to_json(response.into_body()).await;

        // Negative space per case: the internal reason never reaches the body...
        let description = json["error_description"].as_str().unwrap();
        assert!(
            !description.contains(reason),
            "the internal reason must not reach the body"
        );
        // ...and never the telemetry either — only the mapping's fixed warn text does.
        let captured = fragments
            .lock()
            .expect("mutex must not be poisoned")
            .clone();
        assert_absent_plain_and_encoded(&captured, reason);

        bodies.push((json["error"].clone(), json["error_description"].clone()));
    }

    assert!(
        bodies.windows(2).all(|w| w[0] == w[1]),
        "signature/expiry/audience failures must be indistinguishable, got {bodies:?}"
    );
}

// ---------------------------------------------------------------------------
// 3. Hostile upstream end to end: generic body, redacted log
// ---------------------------------------------------------------------------

#[tokio::test]
async fn hostile_upstream_echo_yields_generic_body_and_redacted_log() {
    const UPSTREAM_TOKEN_SENTINEL: &str = "SENTINEL-UPSTREAM-ECHO-TOKEN";

    let server = MockServer::start().await;
    // A hostile upstream echoing the submitted form back: raw, percent-encoded.
    let echo = format!(
        "upstream exploded token={UPSTREAM_TOKEN_SENTINEL} \
         encoded=token%3D1%2F%2F{UPSTREAM_TOKEN_SENTINEL}"
    );
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(500).set_body_string(echo))
        .mount(&server)
        .await;

    let config = oidc_exchange_core::domain::OidcProviderConfig {
        provider_id: "corpus-upstream".to_string(),
        issuer: "https://issuer.example.com".to_string(),
        client_id: "corpus-client".to_string(),
        client_secret: None,
        jwks_uri: Some(format!("{}/jwks.json", server.uri())),
        token_endpoint: Some(format!("{}/token", server.uri())),
        revocation_endpoint: None,
        scopes: Vec::new(),
        additional_params: HashMap::new(),
    };
    let provider =
        oidc_exchange_adapters::oidc::OidcProvider::from_config("corpus-upstream", &config)
            .await
            .expect("build OIDC provider");

    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    providers.insert("corpus-upstream".to_string(), Box::new(provider));

    let (_guard, fragments, _ids) = install_capture();
    let app = build_app(providers);

    let response = post_form(
        app,
        "/token",
        "grant_type=authorization_code&code=irrelevant\
         &redirect_uri=https://client.example.com/callback&provider=corpus-upstream"
            .to_string(),
        None,
    )
    .await;

    // Caller side: only the generic server_error class.
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let json = body_to_json(response.into_body()).await;
    assert_eq!(json["error"], "server_error");
    assert_eq!(json["error_description"], "upstream provider error");
    assert!(
        !json["error_description"]
            .as_str()
            .unwrap()
            .contains(UPSTREAM_TOKEN_SENTINEL),
        "the echoed token must not reach the body"
    );

    // Operator side: the redacted detail survives (markers present), the sentinels do
    // not — raw or percent-decoded.
    let captured = fragments
        .lock()
        .expect("mutex must not be poisoned")
        .clone();
    let joined = captured.join("\n");
    assert!(
        joined.contains("[REDACTED]"),
        "the mapped error log must carry the redacted detail, got {joined}"
    );
    assert_absent_plain_and_encoded(&captured, UPSTREAM_TOKEN_SENTINEL);
}
