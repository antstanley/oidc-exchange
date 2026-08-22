pub mod health;
pub mod internal;
pub mod keys;
pub mod nonce;
pub mod revoke;
pub mod token;
pub mod well_known;

use axum::routing::{get, post};
use axum::Router;

use crate::state::AppState;

pub fn public_routes() -> Router<AppState> {
    Router::new()
        .route("/health", get(health::health_handler))
        .route("/token", post(token::token_handler))
        .route("/revoke", post(revoke::revoke_handler))
        .route("/keys", get(keys::keys_handler))
        .route(
            "/.well-known/openid-configuration",
            get(well_known::openid_config_handler),
        )
}

/// The direct ID-token grant's nonce route. Mounted by `build_router` only
/// when the role serves exchanges **and** `grants.id_token` is enabled — a
/// default-disabled deployment never exposes it.
pub fn nonce_routes() -> Router<AppState> {
    Router::new().route("/nonce", post(nonce::nonce_handler))
}

pub fn internal_routes(state: AppState) -> Router<AppState> {
    internal::router(state)
}
