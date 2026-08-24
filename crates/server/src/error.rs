use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

use oidc_exchange_core::error::Error;

/// Safe, rendered OAuth protocol error classification for response-side consumers.
///
/// This intentionally contains only the public OAuth error code—not an error description,
/// token, or domain error detail—so middleware can record it without inspecting the body.
#[derive(Clone, Copy, Debug)]
pub struct RenderedOAuthErrorCode(pub &'static str);

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
    error_description: String,
}

/// Wrapper around domain errors and route-level errors that implements
/// `IntoResponse` for axum handlers.
#[derive(Debug)]
pub enum ApiError {
    /// A domain error from the core service.
    Domain(Error),
    /// The `grant_type` parameter was not recognized.
    UnsupportedGrantType,
}

impl From<Error> for ApiError {
    fn from(err: Error) -> Self {
        ApiError::Domain(err)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            ApiError::UnsupportedGrantType => {
                let body = ErrorResponse {
                    error: "unsupported_grant_type".to_string(),
                    error_description: "The grant_type parameter is not supported".to_string(),
                };
                oauth_error_response(StatusCode::BAD_REQUEST, body, "unsupported_grant_type")
            }
            ApiError::Domain(err) => {
                let retry_after = match &err {
                    Error::TooManyRequests { retry_after_secs } => Some(*retry_after_secs),
                    _ => None,
                };
                let (status, error_code, description) = map_domain_error(&err);
                let mut response = oauth_error_response(
                    status,
                    ErrorResponse {
                        error: error_code.clone(),
                        error_description: description,
                    },
                    &error_code,
                );
                if let Some(retry_after_secs) = retry_after {
                    let value =
                        axum::http::HeaderValue::from_str(&retry_after_secs.max(1).to_string())
                            .expect(
                                "positive retry-after seconds always form a valid header value",
                            );
                    response
                        .headers_mut()
                        .insert(axum::http::header::RETRY_AFTER, value);
                }
                response
            }
        }
    }
}

fn oauth_error_response(status: StatusCode, body: ErrorResponse, error_code: &str) -> Response {
    let mut response = (status, Json(body)).into_response();
    response
        .extensions_mut()
        .insert(RenderedOAuthErrorCode(match error_code {
            "invalid_request" => "invalid_request",
            "invalid_grant" => "invalid_grant",
            "invalid_token" => "invalid_token",
            "unsupported_grant_type" => "unsupported_grant_type",
            "access_denied" => "access_denied",
            "not_found" => "not_found",
            "conflict" => "conflict",
            "server_error" => "server_error",
            "slow_down" => "slow_down",
            _ => unreachable!("map_domain_error emits a closed OAuth error code set"),
        }));
    response
}

fn map_domain_error(err: &Error) -> (StatusCode, String, String) {
    let (status, error_code, description) = map_domain_error_inner(err);

    // Every mapped class — not only server errors — logs its full internal Display
    // under the request span (see middleware/request_id.rs), so genericising the client
    // body costs the operator nothing: the log carries request_id for correlation and
    // the adapter-composed reason/detail. The client only ever sees `description`.
    if status.is_server_error() {
        tracing::error!(error = %err, status = %status, "internal error mapped to error response");
    } else {
        tracing::warn!(error = %err, status = %status, "client fault mapped to error response");
    }

    // Assert: the published text must never repeat the internal `Display` detail — for
    // any class. The log line above carries the detail server-side; a body that equals
    // the Display would leak the diagnostic to the caller instead.
    assert_ne!(
        description,
        err.to_string(),
        "map_domain_error: client description must stay generic, not the internal detail"
    );
    // Debug-assert the stronger structural invariant: every arm publishes exactly
    // `err.client_description()`, generalising the guard that previously covered only
    // the server_error class. The one deliberate exception is `InvalidRequest`, whose
    // description is the curated parse-boundary reason naming the offending parameter
    // (a closed set — see the arm's comment); everything else stays on the fixed set.
    debug_assert!(
        matches!(err, Error::InvalidRequest { .. }) || description == err.client_description(),
        "map_domain_error: every arm must return err.client_description()"
    );

    (status, error_code, description)
}

fn map_domain_error_inner(err: &Error) -> (StatusCode, String, String) {
    let description_for = |err: &Error| err.client_description().to_string();
    match err {
        Error::InvalidGrant { .. } => (
            StatusCode::BAD_REQUEST,
            "invalid_grant".to_string(),
            description_for(err),
        ),
        Error::InvalidToken { .. } => (
            StatusCode::UNAUTHORIZED,
            "invalid_token".to_string(),
            description_for(err),
        ),
        // `InvalidRequest` reasons are curated at the parse boundary from a
        // closed table of parameter names ("missing required parameter: X",
        // "X is not a parameter of the Y grant") — they never embed caller
        // values or upstream detail, so naming the offending parameter is
        // safe and required by the strict-parse contract (04-http-api.md).
        Error::InvalidRequest { reason } => (
            StatusCode::BAD_REQUEST,
            "invalid_request".to_string(),
            reason.clone(),
        ),
        Error::UnknownProvider { .. } => (
            StatusCode::BAD_REQUEST,
            "invalid_request".to_string(),
            description_for(err),
        ),
        Error::AccessDenied { .. } => (
            StatusCode::FORBIDDEN,
            "access_denied".to_string(),
            description_for(err),
        ),
        Error::UserSuspended { user_id: _ } => (
            StatusCode::FORBIDDEN,
            "access_denied".to_string(),
            description_for(err),
        ),
        Error::Unauthorized { .. } => (
            StatusCode::UNAUTHORIZED,
            "unauthorized".to_string(),
            description_for(err),
        ),
        Error::Conflict { .. } => (
            StatusCode::CONFLICT,
            "conflict".to_string(),
            description_for(err),
        ),
        Error::NotFound { .. } => (
            StatusCode::NOT_FOUND,
            "not_found".to_string(),
            description_for(err),
        ),
        // A throttled caller must back off; the Retry-After header on the
        // auth layer's own 429 carries the authoritative window. This arm is
        // the defensive in-handler mapping for the same variant.
        Error::TooManyRequests {
            retry_after_secs: _,
        } => (
            StatusCode::TOO_MANY_REQUESTS,
            "slow_down".to_string(),
            "too many authentication attempts".to_string(),
        ),
        Error::ProviderError { .. } => (
            StatusCode::BAD_GATEWAY,
            "server_error".to_string(),
            description_for(err),
        ),
        Error::ProviderTimeout { .. } => (
            StatusCode::GATEWAY_TIMEOUT,
            "server_error".to_string(),
            description_for(err),
        ),
        Error::StoreError { .. }
        | Error::KeyError { .. }
        | Error::AuditError { .. }
        | Error::SecurityAuditDurability { .. }
        | Error::SyncError { .. }
        | Error::ConfigError { .. } => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error".to_string(),
            description_for(err),
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use http_body_util::BodyExt;
    use tracing_subscriber::layer::{Context, Layer};
    use tracing_subscriber::prelude::*;

    use super::*;

    /// A single captured `tracing` event: its level plus every field recorded on it,
    /// stringified. Used to assert that `map_domain_error` emits (or does not emit) an
    /// `error`-level event carrying the internal detail, without depending on a specific
    /// log-formatting crate.
    struct CapturedEvent {
        level: tracing::Level,
        fields: HashMap<String, String>,
    }

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

    /// A minimal `tracing` layer that stashes every event emitted while it is the active
    /// subscriber, so a test can inspect level and fields without a full logging pipeline.
    struct EventCaptureLayer {
        events: Arc<Mutex<Vec<CapturedEvent>>>,
    }

    impl<S> Layer<S> for EventCaptureLayer
    where
        S: tracing::Subscriber,
    {
        fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
            let mut fields = HashMap::new();
            event.record(&mut FieldVisitor(&mut fields));
            self.events
                .lock()
                .expect("capture mutex must not be poisoned")
                .push(CapturedEvent {
                    level: *event.metadata().level(),
                    fields,
                });
        }
    }

    /// Runs `map_domain_error` under a subscriber that captures every emitted event, and
    /// returns `(status, error_code, description, all_captured_events)` so a test can
    /// assert on both the response mapping and the log side effect — at any level — in
    /// one place. Callers filter by level: 5xx mappings log at `error`, 4xx at `warn`.
    fn map_domain_error_capturing(err: &Error) -> (StatusCode, String, String, Vec<CapturedEvent>) {
        let events: Arc<Mutex<Vec<CapturedEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let capture = EventCaptureLayer {
            events: events.clone(),
        };
        let subscriber = tracing_subscriber::registry().with(capture);

        let (status, error_code, description) =
            tracing::subscriber::with_default(subscriber, || map_domain_error(err));

        let captured = events
            .lock()
            .expect("capture mutex must not be poisoned")
            .drain(..)
            .collect();

        (status, error_code, description, captured)
    }

    /// Keep only the error-level events from a captured set.
    fn error_level(events: &[CapturedEvent]) -> Vec<&CapturedEvent> {
        events
            .iter()
            .filter(|e| e.level == tracing::Level::ERROR)
            .collect()
    }

    /// Keep only the warn-level events from a captured set.
    fn warn_level(events: &[CapturedEvent]) -> Vec<&CapturedEvent> {
        events
            .iter()
            .filter(|e| e.level == tracing::Level::WARN)
            .collect()
    }

    #[test]
    fn provider_error_logs_internal_detail_and_returns_generic_body() {
        let err = Error::ProviderError {
            provider: "google".to_string(),
            detail: "connection reset by upstream".to_string(),
        };

        let (status, error_code, description, events) = map_domain_error_capturing(&err);
        let error_events = error_level(&events);

        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert_eq!(error_code, "server_error");
        assert_eq!(description, "upstream provider error");
        // Negative-space: the client-facing description must never carry the upstream detail.
        assert!(
            !description.contains("connection reset"),
            "client body must not leak the internal provider detail, got {description:?}"
        );

        assert_eq!(
            error_events.len(),
            1,
            "expected exactly one error-level log for ProviderError, got {}",
            error_events.len()
        );
        let logged = error_events[0]
            .fields
            .get("error")
            .cloned()
            .unwrap_or_default();
        assert!(
            logged.contains("connection reset by upstream"),
            "captured error log must carry the internal detail, got {logged:?}"
        );
    }

    #[test]
    fn provider_timeout_logs_internal_detail_and_returns_generic_body() {
        let err = Error::ProviderTimeout {
            provider: "microsoft".to_string(),
        };

        let (status, error_code, description, events) = map_domain_error_capturing(&err);
        let error_events = error_level(&events);

        assert_eq!(status, StatusCode::GATEWAY_TIMEOUT);
        assert_eq!(error_code, "server_error");
        assert_eq!(description, "upstream provider timeout");

        assert_eq!(
            error_events.len(),
            1,
            "expected exactly one error-level log for ProviderTimeout, got {}",
            error_events.len()
        );
        let logged = error_events[0]
            .fields
            .get("error")
            .cloned()
            .unwrap_or_default();
        assert!(
            logged.contains("microsoft"),
            "captured error log must carry the internal detail, got {logged:?}"
        );
    }

    #[test]
    fn store_error_logs_internal_detail_and_returns_generic_body() {
        let err = Error::StoreError {
            detail: "sqlite: database is locked".to_string(),
        };

        let (status, error_code, description, events) = map_domain_error_capturing(&err);
        let error_events = error_level(&events);

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(error_code, "server_error");
        assert_eq!(description, "internal server error");
        // Negative-space: the client body stays generic, never the store's internal detail.
        assert!(
            !description.contains("sqlite"),
            "client body must not leak the internal store detail, got {description:?}"
        );

        assert_eq!(
            error_events.len(),
            1,
            "expected exactly one error-level log for StoreError, got {}",
            error_events.len()
        );
        let logged = error_events[0]
            .fields
            .get("error")
            .cloned()
            .unwrap_or_default();
        assert!(
            logged.contains("database is locked"),
            "captured error log must carry the internal detail, got {logged:?}"
        );
    }

    #[test]
    fn invalid_grant_returns_generic_body_and_logs_reason_at_warn() {
        let err = Error::InvalidGrant {
            reason: "code already used".to_string(),
        };

        let (status, error_code, description, events) = map_domain_error_capturing(&err);
        let error_events = error_level(&events);
        let warn_events = warn_level(&events);

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(error_code, "invalid_grant");
        // The body is now the fixed client description — never the internal reason.
        assert_eq!(
            description,
            err.client_description(),
            "the published description must be the variant's static text"
        );
        assert!(
            !description.contains("code already used"),
            "client body must not leak the internal validation step, got {description:?}"
        );

        // Negative space retained from the old contract: a client fault must never
        // trigger an error-level log.
        assert!(
            error_events.is_empty(),
            "InvalidGrant must not emit an error-level log, got {} event(s)",
            error_events.len()
        );
        // ...but the operator keeps the diagnostic at warn level.
        assert_eq!(
            warn_events.len(),
            1,
            "InvalidGrant must log its internal detail exactly once at warn, got {}",
            warn_events.len()
        );
        let logged = warn_events[0]
            .fields
            .get("error")
            .cloned()
            .unwrap_or_default();
        assert!(
            logged.contains("code already used"),
            "captured warn log must carry the full internal reason, got {logged:?}"
        );
    }

    #[test]
    fn unknown_kid_is_not_echoed_but_is_logged() {
        let kid_sentinel = "SENTINEL-UNKNOWN-KID";
        let err = Error::InvalidGrant {
            reason: format!("No matching key for kid: {kid_sentinel} (after forced refetch)"),
        };

        let (status, error_code, description, events) = map_domain_error_capturing(&err);

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(error_code, "invalid_grant");
        // Negative space: the caller-supplied `kid` must not be echoed to the response.
        assert!(
            !description.contains(kid_sentinel),
            "unknown kid must never be echoed to the client, got {description:?}"
        );
        // Positive: the operator still sees it in the warn-level diagnostic.
        let warn_events = warn_level(&events);
        assert_eq!(warn_events.len(), 1, "expected one warn event");
        let logged = warn_events[0]
            .fields
            .get("error")
            .cloned()
            .unwrap_or_default();
        assert!(
            logged.contains(kid_sentinel),
            "the warn log must retain the kid for operators, got {logged:?}"
        );
    }

    /// A bad signature, an expired token, and a wrong audience are indistinguishable in
    /// the mapped response — same status, same code, same generic description — while
    /// each keeps its distinct internal Display for the log.
    #[test]
    fn grant_validation_failures_are_indistinguishable_in_responses() {
        let cases = [
            "JWT validation failed: InvalidSignature",
            "JWT validation failed: ExpiredSignature",
            "JWT validation failed: InvalidAudience",
        ];
        let mappings = cases.map(|reason| {
            let err = Error::InvalidGrant {
                reason: reason.to_string(),
            };
            let (status, error_code, description, events) = map_domain_error_capturing(&err);
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert_eq!(error_code, "invalid_grant");
            assert!(!description.contains(reason));
            assert_eq!(warn_level(&events).len(), 1);
            (status, error_code, description)
        });
        assert!(
            mappings.windows(2).all(|w| w[0] == w[1]),
            "signature/expiry/audience failures must be indistinguishable, got {mappings:?}"
        );
    }

    /// The 4xx warn is emitted inside whatever span is active when the mapping runs —
    /// in production that is the per-request span opened by `request_id_layer`, so the
    /// operator event carries the request id for correlation with the generic body.
    #[test]
    fn warn_log_inherits_the_active_request_span() {
        use tracing_subscriber::registry::LookupSpan;

        /// Fields declared on a span, captured at creation so events can be correlated.
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

        struct RequestIdCaptureLayer {
            captured: Arc<Mutex<Option<String>>>,
        }

        impl<S> Layer<S> for RequestIdCaptureLayer
        where
            S: tracing::Subscriber + for<'span> LookupSpan<'span>,
        {
            fn on_new_span(
                &self,
                attrs: &tracing::span::Attributes<'_>,
                id: &tracing::span::Id,
                ctx: Context<'_, S>,
            ) {
                let span = ctx
                    .span(id)
                    .expect("span must exist immediately after creation");
                let mut fields = SpanFields::default();
                attrs.record(&mut FieldVisitor(&mut fields.0));
                span.extensions_mut().insert(fields);
            }

            fn on_event(&self, event: &tracing::Event<'_>, ctx: Context<'_, S>) {
                let Some(scope) = ctx.event_scope(event) else {
                    return;
                };
                for span in scope.from_root() {
                    let extensions = span.extensions();
                    if let Some(fields) = extensions.get::<SpanFields>() {
                        if let Some(request_id) = fields.0.get("request_id") {
                            *self.captured.lock().expect("mutex must not be poisoned") =
                                Some(request_id.clone());
                        }
                    }
                }
            }
        }

        let captured: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let subscriber = tracing_subscriber::registry().with(RequestIdCaptureLayer {
            captured: captured.clone(),
        });

        let err = Error::InvalidRequest {
            reason: "missing required parameter: provider".to_string(),
        };
        let correlation_id = "corr-id-123";

        tracing::subscriber::with_default(subscriber, || {
            let request_span = tracing::info_span!("request", request_id = %correlation_id);
            let _entered = request_span.enter();
            let _mapped = map_domain_error(&err);
        });

        assert_eq!(
            captured.lock().expect("mutex must not be poisoned").clone(),
            Some(correlation_id.to_string()),
            "the warn event must carry the enclosing request span's id"
        );
    }

    async fn response_to_json(response: Response) -> (StatusCode, serde_json::Value) {
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let json = serde_json::from_slice(&bytes).unwrap();
        (status, json)
    }

    #[tokio::test]
    async fn conflict_error_renders_409_with_conflict_code() {
        let err = ApiError::Domain(Error::Conflict {
            detail: "user already registered for (google, sub-123)".to_string(),
        });

        let (status, body) = response_to_json(err.into_response()).await;

        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body["error"], "conflict");
        // The body carries the fixed description only — never the internal detail.
        assert_eq!(
            body["error_description"],
            Error::Conflict {
                detail: String::new()
            }
            .client_description()
        );
        assert!(
            !body["error_description"]
                .as_str()
                .unwrap()
                .contains("sub-123"),
            "conflict body must not leak the internal detail"
        );
    }

    #[tokio::test]
    async fn not_found_error_renders_404_with_not_found_code() {
        let err = ApiError::Domain(Error::NotFound {
            detail: "user abc-123 not found".to_string(),
        });

        let (status, body) = response_to_json(err.into_response()).await;

        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"], "not_found");
        // The body carries the fixed description only — never the internal detail.
        assert_eq!(
            body["error_description"],
            Error::NotFound {
                detail: String::new()
            }
            .client_description()
        );
        assert!(
            !body["error_description"]
                .as_str()
                .unwrap()
                .contains("abc-123"),
            "not-found body must not echo the caller-supplied identifier"
        );
        // Negative-space: NotFound must not be swallowed by the 5xx catch-all.
        assert_ne!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn oauth_error_envelope_enum_includes_conflict_and_stays_closed() {
        let schema_str = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../.specs/canonical-types.schema.json"
        ))
        .expect("failed to read .specs/canonical-types.schema.json");
        let schema: serde_json::Value =
            serde_json::from_str(&schema_str).expect("failed to parse canonical-types schema");

        let enum_values = schema["$defs"]["OAuthErrorEnvelope"]["properties"]["error"]["enum"]
            .as_array()
            .expect("OAuthErrorEnvelope.properties.error.enum must be an array")
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect::<Vec<_>>();

        // Positive: the new wire code is present.
        assert!(
            enum_values.iter().any(|v| v == "conflict"),
            "expected \"conflict\" in the OAuthErrorEnvelope error enum, got {enum_values:?}"
        );

        // Negative-space: the enum is closed — an arbitrary code outside the
        // eight known members is not a valid enum member.
        let bogus = "not_a_real_error_code";
        assert!(
            !enum_values.iter().any(|v| v == bogus),
            "the OAuthErrorEnvelope error enum must stay closed"
        );
        assert_eq!(
            enum_values.len(),
            8,
            "expected exactly eight OAuthErrorEnvelope error codes, got {enum_values:?}"
        );
    }
}
