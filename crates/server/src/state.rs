use std::sync::Arc;

use oidc_exchange_core::config::Config as AppConfig;
use oidc_exchange_core::service::AppService;

#[derive(Clone)]
pub struct AppState {
    pub service: Arc<AppService>,
    pub config: Arc<AppConfig>,
}
