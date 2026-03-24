use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Form;
use serde::Deserialize;

use crate::state::AppState;
use oidc_exchange_core::service::revoke::RevokeRequest;

#[derive(Deserialize)]
pub struct RevokeForm {
    pub token: String,
    pub token_type_hint: Option<String>,
}

pub async fn revoke_handler(
    State(state): State<AppState>,
    Form(form): Form<RevokeForm>,
) -> impl IntoResponse {
    let _ = state
        .service
        .revoke(RevokeRequest {
            token: form.token,
            token_type_hint: form.token_type_hint,
        })
        .await;
    // Per RFC 7009: always return 200
    StatusCode::OK
}
