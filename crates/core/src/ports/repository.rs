use async_trait::async_trait;

use std::collections::HashMap;

use chrono::{DateTime, Utc};

use crate::domain::{NewUser, Session, User, UserPatch};
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
    async fn list_users(&self, offset: u64, limit: u64) -> Result<Vec<User>>;
}

#[async_trait]
pub trait SessionRepository: Send + Sync {
    async fn store_refresh_token(&self, session: &Session) -> Result<()>;
    async fn get_session_by_refresh_token(&self, token_hash: &str) -> Result<Option<Session>>;
    async fn revoke_session(&self, token_hash: &str) -> Result<()>;
    async fn revoke_all_user_sessions(&self, user_id: &str) -> Result<()>;
    async fn count_active_sessions(&self) -> Result<u64>;

    /// Delete all sessions whose `expires_at` is in the past. On stores without native
    /// record expiry (Postgres, SQLite, LMDB) the sweep also reclaims expired
    /// [`crate::domain::SingleUseRecord`]s written by [`Self::put_single_use`], and the
    /// returned count covers both sessions and single-use records; on stores with
    /// native expiry (DynamoDB TTL, Valkey `SET EX`) single-use records need no sweep.
    /// Correctness of [`Self::put_single_use`] / [`Self::take_single_use`] never depends
    /// on this having run — both evaluate `expires_at` themselves.
    async fn cleanup_expired_sessions(&self) -> Result<u64>;

    /// Atomically claim a single-use key: insert-if-absent for a nonce or
    /// assertion-replay marker ([`crate::domain::SingleUseRecord`]).
    ///
    /// Returns `Ok(true)` when *this* call wrote the record, and `Ok(false)` when a live
    /// record already held `key` (someone else claimed it first). A record whose
    /// `expires_at` has passed counts as absent: it does not block the write, so an
    /// expired marker's key is reusable without waiting for a sweep. Exactly one of N
    /// concurrent calls for one live key observes `true`.
    ///
    /// `key` must be a namespaced digest (`"nonce:<sha256hex>"` or
    /// `"assertion:<provider>:[d:]<sha256hex>"`) — storage never holds raw nonce or raw
    /// assertion material.
    async fn put_single_use(&self, key: &str, expires_at: DateTime<Utc>) -> Result<bool>;

    /// Atomically burn a single-use key: remove-and-report.
    ///
    /// Returns `Ok(true)` when a live record was found and is now gone (this call
    /// consumed it), `Ok(false)` when no live record existed — an absent key, an
    /// already-burned key, and an expired one are indistinguishable to the caller. The
    /// check-and-remove is one atomic operation, so exactly one of N concurrent calls
    /// for one live key observes `true`.
    async fn take_single_use(&self, key: &str) -> Result<bool>;
}
