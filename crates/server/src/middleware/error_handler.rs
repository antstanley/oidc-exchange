use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

/// Catch-panic handler for use with `tower_http::catch_panic::CatchPanicLayer`.
///
/// Returns a structured JSON 500 response instead of an empty body when a
/// handler panics. Wired into the middleware stack as:
///
/// ```ignore
/// use tower_http::catch_panic::CatchPanicLayer;
/// app.layer(CatchPanicLayer::custom(panic_handler));
/// ```
pub fn panic_handler(_err: Box<dyn std::any::Any + Send + 'static>) -> Response {
    tracing::error!("handler panicked");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        axum::Json(json!({
            "error": "server_error",
            "error_description": "internal server error"
        })),
    )
        .into_response()
}
