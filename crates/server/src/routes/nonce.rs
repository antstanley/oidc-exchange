use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::json;

use crate::state::AppState;

/// Mint a single-use nonce for the direct ID-token grant.
///
/// Boundary-only: no body to validate (`POST /nonce` takes none), delegate
/// straight to the core's `mint_nonce`, serialise the response. Unauthenticated
/// by necessity — the caller holds no credential yet — and mounted only when
/// `grants.id_token` is enabled, so a default deployment gains no new surface.
/// The response carries the base64url nonce plus `expires_in` seconds; the raw
/// value is returned exactly once here and never logged.
pub async fn nonce_handler(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, crate::error::ApiError> {
    assert!(
        state.config.grants.id_token,
        "POST /nonce must not be mounted when grants.id_token is disabled"
    );

    let minted = state.service.mint_nonce().await?;
    debug_assert!(minted.expires_in > 0 || state.config.grants.nonce_ttl == "0s");
    assert!(!minted.nonce.is_empty(), "minted nonces are never empty");

    Ok(Json(json!({
        "nonce": minted.nonce,
        "expires_in": minted.expires_in,
    })))
}
