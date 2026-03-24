use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::json;

use crate::state::AppState;

pub async fn openid_config_handler(State(state): State<AppState>) -> impl IntoResponse {
    let issuer = &state.config.server.issuer;
    let alg = state.service.signing_algorithm();
    Json(json!({
        "issuer": issuer,
        "jwks_uri": format!("{}/keys", issuer),
        "token_endpoint": format!("{}/token", issuer),
        "revocation_endpoint": format!("{}/revoke", issuer),
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "response_types_supported": ["code"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": [alg]
    }))
}
