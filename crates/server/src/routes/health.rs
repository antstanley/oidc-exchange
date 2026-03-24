use axum::response::IntoResponse;
use axum::Json;
use serde_json::json;

pub async fn health_handler() -> impl IntoResponse {
    Json(json!({"status": "ok"}))
}
