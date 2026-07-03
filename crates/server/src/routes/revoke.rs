use axum::extract::{Extension, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Form;
use serde::Deserialize;

use crate::middleware::audit_context::AuditContext;
use crate::state::AppState;
use oidc_exchange_core::service::revoke::RevokeRequest;

#[derive(Deserialize)]
pub struct RevokeForm {
    pub token: String,
    pub token_type_hint: Option<String>,
}

pub async fn revoke_handler(
    State(state): State<AppState>,
    Extension(audit_ctx): Extension<AuditContext>,
    Form(form): Form<RevokeForm>,
) -> impl IntoResponse {
    let _ = state
        .service
        .revoke(RevokeRequest {
            token: form.token,
            token_type_hint: form.token_type_hint,
            ip_address: audit_ctx.ip_address,
            user_agent: audit_ctx.user_agent,
            device_id: audit_ctx.device_id,
        })
        .await;
    // Per RFC 7009: always return 200
    StatusCode::OK
}
