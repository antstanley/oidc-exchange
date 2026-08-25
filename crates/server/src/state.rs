use std::sync::Arc;

use axum::extract::FromRef;
use oidc_exchange_core::config::Config as AppConfig;
use oidc_exchange_core::ports::RateLimiter;
use oidc_exchange_core::service::AppService;

use crate::middleware::operator_auth::OperatorAuthGate;

#[derive(Clone)]
pub struct AppState {
    pub service: Arc<AppService>,
    pub config: Arc<AppConfig>,
    /// The operator-authentication machinery for `/internal/*`, built once at
    /// startup from validated configuration. `None` whenever this process does
    /// not serve the admin plane; the internal-auth layer is mounted only
    /// where this is `Some`.
    pub operator_auth: Option<Arc<OperatorAuthGate>>,
    /// Selected at startup and retained for future core/router consumers.
    pub rate_limiter: Arc<dyn RateLimiter>,
}

impl FromRef<AppState> for Arc<AppConfig> {
    fn from_ref(state: &AppState) -> Self {
        Arc::clone(&state.config)
    }
}
