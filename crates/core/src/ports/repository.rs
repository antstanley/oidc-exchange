use async_trait::async_trait;

use crate::domain::{NewUser, Session, User, UserPatch};
use crate::error::Result;

#[async_trait]
pub trait Repository: Send + Sync {
    // User operations
    async fn get_user_by_id(&self, user_id: &str) -> Result<Option<User>>;
    async fn get_user_by_external_id(&self, external_id: &str) -> Result<Option<User>>;
    async fn create_user(&self, user: &NewUser) -> Result<User>;
    async fn update_user(&self, user_id: &str, patch: &UserPatch) -> Result<User>;
    async fn delete_user(&self, user_id: &str) -> Result<()>;

    // Session/refresh token operations
    async fn store_refresh_token(&self, session: &Session) -> Result<()>;
    async fn get_session_by_refresh_token(&self, token_hash: &str) -> Result<Option<Session>>;
    async fn revoke_session(&self, token_hash: &str) -> Result<()>;
    async fn revoke_all_user_sessions(&self, user_id: &str) -> Result<()>;
}
