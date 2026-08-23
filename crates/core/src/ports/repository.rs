use async_trait::async_trait;

use std::collections::HashMap;

use crate::domain::{NewUser, Session, User, UserPatch};
use crate::error::Result;
use crate::secret::Secret;

#[async_trait]
pub trait UserRepository: Send + Sync {
    async fn get_user_by_id(&self, user_id: &str) -> Result<Option<User>>;
    async fn get_user_by_external_id(
        &self,
        external_id: &str,
        provider: &str,
    ) -> Result<Option<User>>;
    async fn create_user(&self, user: &NewUser) -> Result<User>;
    async fn update_user(&self, user_id: &str, patch: &UserPatch) -> Result<User>;
    async fn delete_user(&self, user_id: &str) -> Result<()>;
    async fn count_by_status(&self) -> Result<HashMap<String, u64>>;
    async fn list_users(&self, offset: u64, limit: u64) -> Result<Vec<User>>;
}

#[async_trait]
pub trait SessionRepository: Send + Sync {
    async fn store_refresh_token(&self, session: &Session) -> Result<()>;
    /// The token-hash parameters are `&Secret<String>` rather than `&str` so an adapter
    /// that leaves them out of its span `skip(...)` fails to compile instead of publishing
    /// the session lookup key as a span field; the raw digest is reached through
    /// `expose()` only where a store key is built.
    async fn get_session_by_refresh_token(
        &self,
        token_hash: &Secret<String>,
    ) -> Result<Option<Session>>;
    async fn revoke_session(&self, token_hash: &Secret<String>) -> Result<()>;
    async fn revoke_all_user_sessions(&self, user_id: &str) -> Result<()>;
    async fn count_active_sessions(&self) -> Result<u64>;

    /// Delete all sessions whose `expires_at` is in the past.
    /// Returns the number of sessions deleted.
    async fn cleanup_expired_sessions(&self) -> Result<u64>;
}
