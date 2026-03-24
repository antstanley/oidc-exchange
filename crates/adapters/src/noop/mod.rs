use async_trait::async_trait;
use oidc_exchange_core::domain::{AuditEvent, User};
use oidc_exchange_core::error::Result;
use oidc_exchange_core::ports::{AuditLog, UserSync};

/// A no-op audit log that silently discards all events.
///
/// Used as the default when `audit.adapter = "noop"`.
pub struct NoopAuditLog;

impl NoopAuditLog {
    pub fn new() -> Self {
        Self
    }
}

impl Default for NoopAuditLog {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AuditLog for NoopAuditLog {
    async fn emit(&self, _event: &AuditEvent) -> Result<()> {
        Ok(())
    }
}

/// A no-op user sync adapter that does nothing on user lifecycle events.
///
/// Used when `user_sync.enabled = false`.
pub struct NoopUserSync;

impl NoopUserSync {
    pub fn new() -> Self {
        Self
    }
}

impl Default for NoopUserSync {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl UserSync for NoopUserSync {
    async fn notify_user_created(&self, _user: &User) -> Result<()> {
        Ok(())
    }

    async fn notify_user_updated(&self, _user: &User, _changed_fields: &[&str]) -> Result<()> {
        Ok(())
    }

    async fn notify_user_deleted(&self, _user_id: &str) -> Result<()> {
        Ok(())
    }
}
