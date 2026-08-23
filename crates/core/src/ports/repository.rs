use async_trait::async_trait;

use std::collections::HashMap;

use crate::domain::{NewUser, Session, User, UserPage, UserPatch};
use crate::error::Result;

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

    /// Read one bounded page of users.
    ///
    /// `cursor` is the opaque, adapter-issued `next_cursor` from the previous
    /// page; `None` starts the listing. `limit` is the *effective* page size —
    /// the service layer clamps caller input to [`MAX_ADMIN_PAGE_SIZE`] before
    /// this port is reached, and every adapter pushes that bound into the
    /// store rather than materializing an unbounded result. The returned
    /// [`UserPage::next_cursor`] is the only completion signal: adapters may
    /// return a short page that still carries a non-null cursor.
    ///
    /// A cursor is only valid against the adapter that issued it; an
    /// unparseable or tampered cursor is [`Error::InvalidRequest`].
    async fn list_users(&self, cursor: Option<&str>, limit: u32) -> Result<UserPage>;
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
