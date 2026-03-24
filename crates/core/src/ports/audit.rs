use async_trait::async_trait;

use crate::domain::AuditEvent;
use crate::error::Result;

#[async_trait]
pub trait AuditLog: Send + Sync {
    /// Emit an audit event.
    async fn emit(&self, event: &AuditEvent) -> Result<()>;
}
