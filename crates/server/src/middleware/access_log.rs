use axum::http::Request;
use axum::middleware::Next;
use axum::response::Response;

use crate::error::RenderedOAuthErrorCode;
use crate::middleware::audit_context::AuditContext;

/// Emits one safe, request-correlated access log record for every public request.
pub const ACCESS_LOG_REQUEST_ID_HEADER: &str = "x-oidc-exchange-access-log-request-id";
pub const ACCESS_LOG_CLIENT_ADDR_SOURCE_HEADER: &str =
    "x-oidc-exchange-access-log-client-addr-source";

pub async fn access_log_layer(request: Request<axum::body::Body>, next: Next) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let client_addr_source = request
        .extensions()
        .get::<AuditContext>()
        .map(|context| format!("{:?}", context.client_addr.source()).to_lowercase())
        .unwrap_or_else(|| "unknown".to_owned());
    let request_id = request
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("unknown")
        .to_owned();
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        ACCESS_LOG_REQUEST_ID_HEADER,
        request_id.parse().expect("validated request id header"),
    );
    response.headers_mut().insert(
        ACCESS_LOG_CLIENT_ADDR_SOURCE_HEADER,
        client_addr_source
            .parse()
            .expect("fixed client address source value"),
    );
    let error = response
        .extensions()
        .get::<RenderedOAuthErrorCode>()
        .map(|error| error.0);
    tracing::info!(
        request_id = %request_id,
        method = %method,
        path = %path,
        status = %response.status(),
        client_addr_source = %client_addr_source,
        error = ?error,
        "public access"
    );
    response
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::response::IntoResponse;
    use axum::routing::post;
    use axum::Router;
    use oidc_exchange_core::error::Error;
    use serde::Serialize;
    use tower::ServiceExt;

    use crate::error::ApiError;
    use tracing_subscriber::layer::{Context, Layer};
    use tracing_subscriber::prelude::*;

    use super::*;

    struct FieldVisitor<'a>(&'a mut HashMap<String, String>);

    impl tracing::field::Visit for FieldVisitor<'_> {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            self.0.insert(field.name().to_owned(), format!("{value:?}"));
        }

        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            self.0.insert(field.name().to_owned(), value.to_owned());
        }
    }

    struct EventCaptureLayer {
        events: Arc<Mutex<Vec<HashMap<String, String>>>>,
    }

    impl<S> Layer<S> for EventCaptureLayer
    where
        S: tracing::Subscriber,
    {
        fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
            let mut fields = HashMap::new();
            event.record(&mut FieldVisitor(&mut fields));
            if fields.get("message").map(String::as_str) != Some("public access") {
                return;
            }
            self.events
                .lock()
                .expect("capture mutex must not be poisoned")
                .push(fields);
        }
    }

    #[derive(Serialize)]
    struct OAuthFailure {
        error: &'static str,
        error_description: &'static str,
    }

    #[tokio::test]
    async fn logs_core_too_many_requests_as_slow_down_without_reading_response_body() {
        let app = Router::new()
            .route(
                "/token",
                post(|| async {
                    ApiError::Domain(Error::TooManyRequests {
                        retry_after_secs: 30,
                    })
                }),
            )
            .route_layer(axum::middleware::from_fn(access_log_layer));
        let events = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::registry().with(EventCaptureLayer {
            events: events.clone(),
        });
        let _guard = tracing::subscriber::set_default(subscriber);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/token")
                    .header("x-request-id", "slow-down-access-log-test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("router response");

        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.headers()["retry-after"], "30");
        let events = events.lock().expect("capture mutex must not be poisoned");
        assert_eq!(
            events.len(),
            1,
            "expected one public access event: {events:?}"
        );
        assert_eq!(
            events[0].get("error"),
            Some(&"Some(\"slow_down\")".to_owned())
        );
        assert!(
            !events[0]
                .values()
                .any(|value| value.contains("too many authentication attempts")),
            "access record must not log OAuth error detail: {events:?}"
        );
    }

    #[tokio::test]
    async fn logs_rendered_oauth_error_code_without_reading_response_body() {
        let app = Router::new()
            .route(
                "/token",
                post(|| async {
                    let mut response = (
                        StatusCode::BAD_REQUEST,
                        axum::Json(OAuthFailure {
                            error: "invalid_grant",
                            error_description: "sensitive token detail",
                        }),
                    )
                        .into_response();
                    response
                        .extensions_mut()
                        .insert(RenderedOAuthErrorCode("invalid_grant"));
                    response
                }),
            )
            .route_layer(axum::middleware::from_fn(access_log_layer));
        let events = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::registry().with(EventCaptureLayer {
            events: events.clone(),
        });
        let _guard = tracing::subscriber::set_default(subscriber);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/token")
                    .header("x-request-id", "access-log-test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("router response");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let events = events.lock().expect("capture mutex must not be poisoned");
        assert_eq!(
            events.len(),
            1,
            "expected one public access event: {events:?}"
        );
        assert_eq!(
            events[0].get("error"),
            Some(&"Some(\"invalid_grant\")".to_owned())
        );
        assert!(
            !events[0]
                .values()
                .any(|value| value.contains("sensitive token detail")),
            "access record must not log OAuth error detail: {events:?}"
        );
    }
}
