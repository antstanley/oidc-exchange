use axum::extract::State;
use axum::response::IntoResponse;
use axum::Form;
use axum::Json;
use serde::Deserialize;

use crate::error::ApiError;
use crate::state::AppState;
use oidc_exchange_core::error::Error;
use oidc_exchange_core::service::exchange::ExchangeRequest;
use oidc_exchange_core::service::refresh::RefreshRequest;

#[derive(Deserialize)]
pub struct TokenForm {
    pub grant_type: String,
    pub code: Option<String>,
    pub redirect_uri: Option<String>,
    pub provider: Option<String>,
    pub refresh_token: Option<String>,
    pub id_token: Option<String>,
}

pub async fn token_handler(
    State(state): State<AppState>,
    Form(form): Form<TokenForm>,
) -> Result<impl IntoResponse, ApiError> {
    match form.grant_type.as_str() {
        "authorization_code" | "id_token" => {
            let provider = form.provider.ok_or_else(|| Error::InvalidRequest {
                reason: "missing required parameter: provider".to_string(),
            })?;
            let result = state
                .service
                .exchange(ExchangeRequest {
                    code: form.code,
                    redirect_uri: form.redirect_uri,
                    id_token: form.id_token,
                    provider,
                })
                .await?;
            Ok(Json(result))
        }
        "refresh_token" => {
            let refresh_token = form.refresh_token.ok_or_else(|| Error::InvalidRequest {
                reason: "missing required parameter: refresh_token".to_string(),
            })?;
            let result = state
                .service
                .refresh(RefreshRequest { refresh_token })
                .await?;
            Ok(Json(result))
        }
        _ => Err(ApiError::UnsupportedGrantType),
    }
}
