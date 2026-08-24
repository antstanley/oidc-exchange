//! Session-store maintenance — the one shared sweep entry point that both
//! callers of [`SessionRepository::cleanup_expired_sessions`] drive.
//!
//! `04-http-api.md` → Bootstrap step 7 has the server's session reaper call
//! the port on `session_repository.cleanup_interval`, and Internal routes has
//! `POST /internal/sessions/cleanup` call it once for runtimes that cannot
//! host a periodic task. Both go through this `AppService` method so there is
//! exactly one place that names the port operation and no handler or runtime
//! loop touches the session repository directly.

use crate::error::Result;
use crate::service::AppService;

impl AppService {
    /// Delete every expired row from the session store — live sessions past
    /// their absolute `expires_at` and retained retirement records past their
    /// reuse-retention deadline alike — returning the combined number of rows
    /// deleted.
    ///
    /// Safe to run at any cadence and concurrently with a scheduled reaper:
    /// the sweep mutates nothing but expired rows. On the natively-expiring
    /// stores (DynamoDB TTL, Valkey key expiry) it is a cheap backstop for
    /// whatever native expiry has not yet reaped; see `06-configuration.md`
    /// → `[session_repository]`.
    pub async fn cleanup_expired_sessions(&self) -> Result<u64> {
        self.session_repo.cleanup_expired_sessions().await
    }
}
