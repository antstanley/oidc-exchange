pub mod health;
pub mod internal;
pub mod keys;
pub mod nonce;
pub mod revoke;
pub mod token;
pub mod well_known;

use axum::middleware;
use axum::routing::{get, post};
use axum::Router;

use crate::middleware::cache_control::no_store_layer;
use crate::state::AppState;

pub fn public_routes() -> Router<AppState> {
    // Credential-bearing routes: requests carry credentials and the /token
    // response body *is* one, so every response in this group — success or
    // OAuth error envelope — is marked non-storable by a route-scoped layer
    // (RFC 6749 §5.1/§5.2). Mounting, not handler memory: a future
    // credential-returning route inherits the directives by joining this
    // group. /keys and discovery keep their own (cacheable) policy outside.
    let credential_routes = Router::new()
        .route("/token", post(token::token_handler))
        .route("/revoke", post(revoke::revoke_handler))
        .layer(middleware::from_fn(no_store_layer));

    Router::new()
        .route("/health", get(health::health_handler))
        .route("/keys", get(keys::keys_handler))
        .route(
            "/.well-known/openid-configuration",
            get(well_known::openid_config_handler),
        )
        .merge(credential_routes)
}

/// The direct ID-token grant's nonce route. Mounted by `build_router` only
/// when the role serves exchanges **and** `grants.id_token` is enabled — a
/// default-disabled deployment never exposes it.
pub fn nonce_routes() -> Router<AppState> {
    Router::new().route("/nonce", post(nonce::nonce_handler))
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
