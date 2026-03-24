use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use subtle::ConstantTimeEq;

use crate::state::AppState;

/// Middleware that validates the `Authorization: Bearer <secret>` header
/// against the configured internal API shared secret using constant-time
/// comparison.
pub async fn internal_auth_layer(
    State(state): State<AppState>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let secret = match state.config.internal_api.shared_secret.as_deref() {
        Some(s) => s,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "unauthorized", "error_description": "internal API not configured"})),
            )
                .into_response();
        }
    };

    let auth_header = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    match auth_header {
        Some(token) if constant_time_eq(token.as_bytes(), secret.as_bytes()) => {
            next.run(request).await
        }
        _ => (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "unauthorized", "error_description": "invalid or missing bearer token"})),
        )
            .into_response(),
    }
}

/// Constant-time byte comparison using the `subtle` crate.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        // Still do a comparison to avoid timing leak on length,
        // but we know it will be false.
        let _ = a.ct_eq(&vec![0u8; a.len()]);
        return false;
    }
    a.ct_eq(b).into()
}
