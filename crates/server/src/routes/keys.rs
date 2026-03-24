use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::json;

use crate::error::ApiError;
use crate::state::AppState;

pub async fn keys_handler(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    let jwk = state.service.public_jwk().await?;
    Ok(Json(json!({"keys": [jwk]})))
}
