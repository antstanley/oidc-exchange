use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use oidc_exchange_core::service::audit_sink_degraded;
use serde_json::json;

pub async fn health_handler() -> impl IntoResponse {
    if audit_sink_degraded() {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"status": "degraded", "reason": "audit_sink_failure"})),
        )
    } else {
        (StatusCode::OK, Json(json!({"status": "ok"})))
    }
}
