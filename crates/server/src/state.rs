use std::sync::Arc;

use axum::extract::FromRef;
use oidc_exchange_core::config::AppConfig;
use oidc_exchange_core::ports::RateLimiter;
use oidc_exchange_core::service::AppService;

#[derive(Clone)]
pub struct AppState {
    pub service: Arc<AppService>,
    pub config: Arc<AppConfig>,
    /// Selected at startup and retained for future core/router consumers.
    pub rate_limiter: Arc<dyn RateLimiter>,
}

impl FromRef<AppState> for Arc<AppConfig> {
    fn from_ref(state: &AppState) -> Self {
        Arc::clone(&state.config)
    }
}
