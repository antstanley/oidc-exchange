use async_trait::async_trait;

use crate::domain::User;
use crate::error::Result;

#[async_trait]
pub trait UserSync: Send + Sync {
    async fn notify_user_created(&self, user: &User) -> Result<()>;
    async fn notify_user_updated(&self, user: &User, changed_fields: &[&str]) -> Result<()>;
    async fn notify_user_deleted(&self, user_id: &str) -> Result<()>;
}
