use async_trait::async_trait;

use std::collections::HashMap;

use crate::domain::{NewUser, Session, User, UserPatch};
use crate::error::Result;

#[async_trait]
pub trait UserRepository: Send + Sync {
    async fn get_user_by_id(&self, user_id: &str) -> Result<Option<User>>;
    async fn get_user_by_external_id(&self, external_id: &str) -> Result<Option<User>>;
    async fn create_user(&self, user: &NewUser) -> Result<User>;
    async fn update_user(&self, user_id: &str, patch: &UserPatch) -> Result<User>;
    async fn delete_user(&self, user_id: &str) -> Result<()>;
    async fn count_by_status(&self) -> Result<HashMap<String, u64>>;
    async fn list_users(&self, offset: u64, limit: u64) -> Result<Vec<User>>;
}

#[async_trait]
pub trait SessionRepository: Send + Sync {
    async fn store_refresh_token(&self, session: &Session) -> Result<()>;
    async fn get_session_by_refresh_token(&self, token_hash: &str) -> Result<Option<Session>>;
    async fn revoke_session(&self, token_hash: &str) -> Result<()>;
    async fn revoke_all_user_sessions(&self, user_id: &str) -> Result<()>;
    async fn count_active_sessions(&self) -> Result<u64>;

    /// Delete all sessions whose `expires_at` is in the past.
    /// Returns the number of sessions deleted.
    async fn cleanup_expired_sessions(&self) -> Result<u64>;
}
