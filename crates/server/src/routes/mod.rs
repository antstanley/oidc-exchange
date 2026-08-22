pub mod health;
pub mod internal;
pub mod keys;
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

/// Mount the internal API under its `/internal` prefix.
///
/// `nest` (rather than full-path routes plus a router-wide layer) is what
/// keeps the operator-auth layer confined to the `/internal` subtree: the
/// admin listener's `/health` route and its fallback never pass through it,
/// so unmatched paths on the admin plane 404 at the routing level instead of
/// surfacing an authentication rejection for routes that do not exist.
pub fn internal_routes(state: AppState) -> Router<AppState> {
    Router::new().nest("/internal", internal::router(state))
}
