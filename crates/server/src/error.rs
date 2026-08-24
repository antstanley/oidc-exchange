use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

use oidc_exchange_core::error::Error;

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
                (StatusCode::BAD_REQUEST, Json(body)).into_response()
            }
            ApiError::Domain(err) => {
                let (status, error_code, description) = map_domain_error(&err);
                let body = ErrorResponse {
                    error: error_code,
                    error_description: description,
                };
                (status, Json(body)).into_response()
            }
        }
    }
}

fn map_domain_error(err: &Error) -> (StatusCode, String, String) {
    let (status, error_code, description) = map_domain_error_inner(err);

    // The server_error class covers upstream/provider/store/infra failures; every other
    // arm maps to a 4xx/409/404 client fault and must never take this branch.
    if error_code == "server_error" {
        // Assert: every arm that sets error_code to "server_error" maps to a 5xx status —
        // catches a future arm that mislabels a client fault as server_error (and would
        // otherwise silently skip the detail log below).
        assert!(
            status.is_server_error(),
            "map_domain_error: server_error class must map to a 5xx status, got {status}"
        );
        // Assert: the client-facing description never repeats the internal `Display` detail
        // — the log line below carries the detail server-side, so the body must stay generic
        // or the detail leaks to the caller through the response instead.
        assert_ne!(
            description,
            err.to_string(),
            "map_domain_error: server_error description must stay generic, not the internal detail"
        );
        // Logged inside the request span (see middleware/request_id.rs), so this event
        // carries `request_id` for correlation; the client only ever sees `description`.
        tracing::error!(error = %err, status = %status, "internal error mapped to server_error response");
    }

    (status, error_code, description)
}

fn map_domain_error_inner(err: &Error) -> (StatusCode, String, String) {
    match err {
        Error::InvalidGrant { reason } => (
            StatusCode::BAD_REQUEST,
            "invalid_grant".to_string(),
            reason.clone(),
        ),
        Error::InvalidToken { reason } => (
            StatusCode::UNAUTHORIZED,
            "invalid_token".to_string(),
            reason.clone(),
        ),
        Error::InvalidRequest { reason } => (
            StatusCode::BAD_REQUEST,
            "invalid_request".to_string(),
            reason.clone(),
        ),
        Error::UnknownProvider { provider } => (
            StatusCode::BAD_REQUEST,
            "invalid_request".to_string(),
            format!("unknown provider: {}", provider),
        ),
        Error::AccessDenied { reason } => (
            StatusCode::FORBIDDEN,
            "access_denied".to_string(),
            reason.clone(),
        ),
        Error::UserSuspended { user_id: _ } => (
            StatusCode::FORBIDDEN,
            "access_denied".to_string(),
            "user account is suspended".to_string(),
        ),
        Error::Unauthorized { reason } => (
            StatusCode::UNAUTHORIZED,
            "unauthorized".to_string(),
            reason.clone(),
        ),
        Error::Conflict { detail } => {
            (StatusCode::CONFLICT, "conflict".to_string(), detail.clone())
        }
        Error::NotFound { detail } => (
            StatusCode::NOT_FOUND,
            "not_found".to_string(),
            detail.clone(),
        ),
        Error::ProviderError { .. } => (
            StatusCode::BAD_GATEWAY,
            "server_error".to_string(),
            "upstream provider error".to_string(),
        ),
        Error::ProviderTimeout { .. } => (
            StatusCode::GATEWAY_TIMEOUT,
            "server_error".to_string(),
            "upstream provider timeout".to_string(),
        ),
        Error::StoreError { .. }
        | Error::KeyError { .. }
        | Error::AuditError { .. }
        | Error::SyncError { .. }
        | Error::ConfigError { .. } => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error".to_string(),
            "internal server error".to_string(),
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
    /// returns `(status, error_code, description, error_level_events)` so a test can assert
    /// on both the response mapping and the log side effect in one place.
    fn map_domain_error_capturing(err: &Error) -> (StatusCode, String, String, Vec<CapturedEvent>) {
        let events: Arc<Mutex<Vec<CapturedEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let capture = EventCaptureLayer {
            events: events.clone(),
        };
        let subscriber = tracing_subscriber::registry().with(capture);

        let (status, error_code, description) =
            tracing::subscriber::with_default(subscriber, || map_domain_error(err));

        let error_events = events
            .lock()
            .expect("capture mutex must not be poisoned")
            .drain(..)
            .filter(|e| e.level == tracing::Level::ERROR)
            .collect();

        (status, error_code, description, error_events)
    }

    #[test]
    fn provider_error_logs_internal_detail_and_returns_generic_body() {
        let err = Error::ProviderError {
            provider: "google".to_string(),
            detail: "connection reset by upstream".to_string(),
        };

        let (status, error_code, description, error_events) = map_domain_error_capturing(&err);

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

        let (status, error_code, description, error_events) = map_domain_error_capturing(&err);

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

        let (status, error_code, description, error_events) = map_domain_error_capturing(&err);

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
    fn invalid_grant_emits_no_server_error_detail_log() {
        let err = Error::InvalidGrant {
            reason: "code already used".to_string(),
        };

        let (status, error_code, description, error_events) = map_domain_error_capturing(&err);

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(error_code, "invalid_grant");
        assert_eq!(description, "code already used");
        // Negative-space: a client-fault error must never trigger the server_error detail
        // log — only the server_error class does.
        assert!(
            error_events.is_empty(),
            "InvalidGrant must not emit an error-level log, got {} event(s)",
            error_events.len()
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
        assert_eq!(
            body["error_description"],
            "user already registered for (google, sub-123)"
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
        assert_eq!(body["error_description"], "user abc-123 not found");
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
