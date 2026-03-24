pub mod exchange;

use std::collections::HashMap;

use crate::config::AppConfig;
use crate::ports::{AuditLog, IdentityProvider, KeyManager, Repository, UserSync};

pub struct AppService {
    pub(crate) repo: Box<dyn Repository>,
    pub(crate) keys: Box<dyn KeyManager>,
    pub(crate) audit: Box<dyn AuditLog>,
    pub(crate) user_sync: Box<dyn UserSync>,
    pub(crate) providers: HashMap<String, Box<dyn IdentityProvider>>,
    pub(crate) config: AppConfig,
}

impl AppService {
    pub fn new(
        repo: Box<dyn Repository>,
        keys: Box<dyn KeyManager>,
        audit: Box<dyn AuditLog>,
        user_sync: Box<dyn UserSync>,
        providers: HashMap<String, Box<dyn IdentityProvider>>,
        config: AppConfig,
    ) -> Self {
        Self {
            repo,
            keys,
            audit,
            user_sync,
            providers,
            config,
        }
    }
}
