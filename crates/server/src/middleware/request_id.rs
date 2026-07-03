use axum::http::Request;
use axum::middleware::Next;
use axum::response::IntoResponse;
use tracing::Instrument;

/// Header name carrying the request id, reused inbound and always echoed outbound.
const REQUEST_ID_HEADER: &str = "x-request-id";

/// Middleware that ensures every request/response carries an `X-Request-Id` header and that
/// every log emitted while handling the request is correlated to it.
///
/// If the incoming request already contains a non-empty `X-Request-Id` header it is reused;
/// otherwise (header absent, empty, or not valid UTF-8) a new UUID v4 is generated. This
/// middleware is the innermost layer, so its per-request `info_span` carrying `request_id`
/// (plus `method` and `path`) wraps the handler only (not the outer middleware stack), so
/// every log emitted by the handler — including the `server_error` detail log — inherits the field,
/// making request-id correlation real. If an outer span is already open when this
/// middleware runs (e.g. a future tower OTEL request-span layer sitting above it),
/// `request_id` is recorded on that span instead of opening a nested duplicate — per the
/// "spans merge across layers, never nest" decision. Either way the value is propagated
/// back on the response header.
pub async fn request_id_layer(request: Request<axum::body::Body>, next: Next) -> impl IntoResponse {
    let request_id = request
        .headers()
        .get(REQUEST_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
        .map(String::from)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    // Precondition: whether reused or generated, the id must never be blank. An empty
    // header value is treated as absent (filtered out above) so this holds by construction
    // rather than by chance — a blank id would still satisfy `to_str()` but is not a usable
    // correlation value.
    assert!(
        !request_id.is_empty(),
        "request_id must be non-empty: generated as a UUID v4 or reused from the request header"
    );

    let method = request.method().clone();
    let path = request.uri().path().to_string();

    let outer_span = tracing::Span::current();
    let mut response = if outer_span.is_disabled() {
        // No span is open yet (e.g. no OTEL request-span layer sits above this one), so
        // this middleware owns the per-request span for the handler it wraps.
        let span = tracing::info_span!(
            "request",
            request_id = %request_id,
            method = %method,
            path = %path,
        );
        next.run(request).instrument(span).await
    } else {
        // An outer span already exists (e.g. an OTEL request-span layer above this one) —
        // fold request_id into it rather than nesting a second span.
        outer_span.record("request_id", request_id.as_str());
        next.run(request).await
    };

    if let Ok(value) = request_id.parse() {
        response.headers_mut().insert(REQUEST_ID_HEADER, value);
    }
    // Postcondition: the response must always carry the echoed id before it leaves this
    // middleware, regardless of which branch above produced it.
    assert!(
        response.headers().get(REQUEST_ID_HEADER).is_some(),
        "response must carry the echoed x-request-id header"
    );

    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{HeaderValue, Request, StatusCode};
    use axum::middleware;
    use axum::routing::get;
    use axum::Router;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use tower::ServiceExt;
    use tracing_subscriber::layer::{Context, Layer};
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::registry::LookupSpan;

    async fn ok_handler() -> &'static str {
        "ok"
    }

    fn app() -> Router {
        Router::new()
            .route("/", get(ok_handler))
            .layer(middleware::from_fn(request_id_layer))
    }

    /// Fields recorded on a single span, captured for inspection by `RequestIdCaptureLayer`.
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

    /// A minimal `tracing` layer that, for every event, walks the currently active span
    /// scope and stashes the `request_id` field of the innermost span that declares one.
    /// Used to prove that a log emitted inside a handler wrapped by `request_id_layer`
    /// actually inherits the per-request span's `request_id` field, rather than the
    /// pre-fix no-op `record` call that silently dropped it.
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

        fn on_record(
            &self,
            id: &tracing::span::Id,
            values: &tracing::span::Record<'_>,
            ctx: Context<'_, S>,
        ) {
            let span = ctx
                .span(id)
                .expect("span must exist when recording new values");
            let mut extensions = span.extensions_mut();
            if let Some(fields) = extensions.get_mut::<SpanFields>() {
                values.record(&mut FieldVisitor(&mut fields.0));
            }
        }

        fn on_event(&self, event: &tracing::Event<'_>, ctx: Context<'_, S>) {
            let Some(scope) = ctx.event_scope(event) else {
                return;
            };
            for span in scope.from_root() {
                let extensions = span.extensions();
                if let Some(fields) = extensions.get::<SpanFields>() {
                    if let Some(request_id) = fields.0.get("request_id") {
                        *self
                            .captured
                            .lock()
                            .expect("capture mutex must not be poisoned") =
                            Some(request_id.clone());
                    }
                }
            }
        }
    }

    /// A handler that emits a log event, used to prove request-id correlation: with
    /// `RequestIdCaptureLayer` installed, this event's enclosing span must carry the same
    /// `request_id` the middleware put on the response header.
    async fn logging_handler() -> &'static str {
        tracing::info!("handling request");
        "ok"
    }

    fn app_with_logging_handler() -> Router {
        Router::new()
            .route("/", get(logging_handler))
            .layer(middleware::from_fn(request_id_layer))
    }

    #[tokio::test]
    async fn generates_request_id_when_absent() {
        let response = app()
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let id = response
            .headers()
            .get("x-request-id")
            .expect("should have x-request-id header")
            .to_str()
            .unwrap();

        // Should be a valid UUID v4 (36 chars with dashes)
        assert_eq!(id.len(), 36);
        assert!(uuid::Uuid::parse_str(id).is_ok());
    }

    #[tokio::test]
    async fn preserves_existing_request_id() {
        let response = app()
            .oneshot(
                Request::get("/")
                    .header("x-request-id", "custom-id-123")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let id = response
            .headers()
            .get("x-request-id")
            .expect("should have x-request-id header")
            .to_str()
            .unwrap();

        assert_eq!(id, "custom-id-123");
    }

    #[tokio::test]
    async fn generates_valid_request_id_when_header_is_malformed() {
        // Bytes that are a legal HTTP header value (no NUL/CR/LF) but not valid UTF-8, so
        // `to_str()` fails and the middleware must fall back to generating a fresh id —
        // never echo the un-parseable bytes.
        let malformed = HeaderValue::from_bytes(&[0xff, 0xfe]).expect("valid raw header bytes");

        let response = app()
            .oneshot(
                Request::get("/")
                    .header("x-request-id", malformed)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let id = response
            .headers()
            .get("x-request-id")
            .expect("should have x-request-id header")
            .to_str()
            .expect("generated id must be valid UTF-8/ASCII");

        assert_eq!(id.len(), 36);
        assert!(uuid::Uuid::parse_str(id).is_ok());
    }

    #[tokio::test]
    async fn generates_valid_request_id_when_header_is_empty() {
        // A present-but-blank `x-request-id` header is client-triggerable (any caller can
        // send `X-Request-Id:` with no value). It must not be treated as a usable id — the
        // middleware must fall back to a generated UUID rather than propagating an empty
        // string that would trip the non-empty precondition and turn into a 500.
        let response = app()
            .oneshot(
                Request::get("/")
                    .header("x-request-id", HeaderValue::from_static(""))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let id = response
            .headers()
            .get("x-request-id")
            .expect("should have x-request-id header")
            .to_str()
            .expect("generated id must be valid UTF-8/ASCII");

        assert_eq!(id.len(), 36);
        assert!(uuid::Uuid::parse_str(id).is_ok());
    }

    #[tokio::test]
    async fn handler_log_carries_generated_request_id() {
        let captured: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let capture_layer = RequestIdCaptureLayer {
            captured: captured.clone(),
        };
        let subscriber = tracing_subscriber::registry().with(capture_layer);
        // Stays active for the whole async body below (single-threaded `#[tokio::test]`
        // runtime keeps every poll on this OS thread), unlike `with_default`'s closure scope.
        let _guard = tracing::subscriber::set_default(subscriber);

        let response = app_with_logging_handler()
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let echoed_id = response
            .headers()
            .get("x-request-id")
            .expect("should have x-request-id header")
            .to_str()
            .unwrap()
            .to_string();

        let captured_id = captured.lock().unwrap().clone();
        assert_eq!(
            captured_id,
            Some(echoed_id),
            "the log emitted inside the handler must carry the same request_id as the response header"
        );
    }

    #[tokio::test]
    async fn handler_log_carries_reused_request_id() {
        let captured: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let capture_layer = RequestIdCaptureLayer {
            captured: captured.clone(),
        };
        let subscriber = tracing_subscriber::registry().with(capture_layer);
        let _guard = tracing::subscriber::set_default(subscriber);

        let response = app_with_logging_handler()
            .oneshot(
                Request::get("/")
                    .header("x-request-id", "reused-id-456")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let echoed_id = response
            .headers()
            .get("x-request-id")
            .expect("should have x-request-id header")
            .to_str()
            .unwrap()
            .to_string();
        assert_eq!(echoed_id, "reused-id-456");

        let captured_id = captured.lock().unwrap().clone();
        assert_eq!(
            captured_id,
            Some("reused-id-456".to_string()),
            "the log emitted inside the handler must carry the reused request_id, not a generated one"
        );
    }
}
