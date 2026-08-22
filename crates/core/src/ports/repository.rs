use async_trait::async_trait;

use std::collections::HashMap;

use crate::domain::{NewUser, RefreshResolution, Session, User, UserPatch};
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

/// The persistent-session port.
///
/// A refresh token belongs to a **family**: every generation descended from
/// one sign-in shares a [`Session::family_id`], and a family has exactly one
/// live generation at any instant. Redemption is a state transition on this
/// port — classify ([`resolve_refresh_token`](Self::resolve_refresh_token)),
/// then atomically rotate ([`rotate_refresh_token`](Self::rotate_refresh_token))
/// — not a read followed by writes from the service, which would race on every
/// backend.
///
/// Five obligations attach to this port. They are contract, not description:
/// an adapter either meets them or it does not ship. The shared conformance
/// suite (`oidc_exchange_test_utils::session_contract`) asserts each one
/// against every implementation, including `MockRepository`.
///
/// | | Obligation |
/// |---|---|
/// | **SR1** | **Consistency.** [`resolve_refresh_token`](Self::resolve_refresh_token) is strongly consistent with the most recent write. Its negative and retired answers *are* security outcomes — an eventually consistent read turns a revoked token into a live one and a reuse alarm into a silent rejection. |
/// | **SR2** | **Atomicity.** [`rotate_refresh_token`](Self::rotate_refresh_token) applies its three effects — delete the live session, write the retirement record, install the replacement — as one atomic unit conditioned on `live_hash` still being live, or applies none of them. A partial application either strands the old generation as still-valid or locks the holder out of a session they legitimately hold. |
/// | **SR3** | **Single live generation.** At most one generation of a family is live at any instant, under concurrent redemption. Two callers redeeming the same hash produce exactly one `true` return. |
/// | **SR4** | **Retirement durability.** By the time a rotation is observable, the retirement record it wrote is readable. A rotation whose replacement is visible before its retirement record leaves a window in which reuse reads as unknown. |
/// | **SR5** | **Revocation completeness.** [`revoke_family`](Self::revoke_family) removes the family's live generation and every retained retirement record, and returns the count removed, or it errors. [`revoke_all_user_sessions`](Self::revoke_all_user_sessions) gives the same removal guarantee across all of a user's families (its `Result<()>` signature is unchanged). Neither reports success for work it did not do. |
///
/// Identifiers passed to this port are minted by core: refresh-token hashes
/// are SHA-256 hex digests of opaque tokens (never the raw tokens), and family
/// ids are `fam_` + lowercase ULID
/// ([`crate::domain::is_valid_family_id`]). Implementations must never log,
/// audit, or embed raw refresh tokens; hashes are the only durable form.
#[async_trait]
pub trait SessionRepository: Send + Sync {
    /// Write one generation's row. At exchange this is the generation-0 row of
    /// a brand-new family; the caller owns assigning `family_id`, `generation`,
    /// and `rotated_at`.
    async fn store_refresh_token(&self, session: &Session) -> Result<()>;

    /// Return the live session stored under `token_hash`, if any. Retirement
    /// records are not sessions: a retired hash yields `None` here. Remains
    /// for `/revoke`, which needs only liveness.
    async fn get_session_by_refresh_token(&self, token_hash: &str) -> Result<Option<Session>>;

    /// Classify `token_hash` against the family state — the live generations
    /// and the retained [`RetiredRefreshToken`] records. Strongly consistent
    /// with the most recent write (SR1).
    ///
    /// The port classifies; it does not decide policy. [`RefreshResolution::Superseded`]
    /// is a storage fact — the successor pointer still names the live
    /// generation — and the grace window that turns it into either a rotation
    /// or a reuse alarm is evaluated once in the core against configuration,
    /// not once per adapter.
    async fn resolve_refresh_token(&self, token_hash: &str) -> Result<RefreshResolution>;

    /// Compare-and-swap one redemption: delete the live session named by
    /// `live_hash`, write a [`RetiredRefreshToken`] for it naming
    /// `replacement.refresh_token_hash` as successor, and install `replacement`
    /// — as one atomic unit conditioned on `live_hash` still being that
    /// family's live generation (SR2).
    ///
    /// Returns `true` when the swap was applied, `false` when the condition
    /// failed because a concurrent redemption moved the live generation first
    /// (SR3). A `false` return must leave the store byte-identical: no partial
    /// delete, no orphaned retirement record, no installed replacement. The
    /// replacement inherits the family's absolute `expires_at`; rotation never
    /// moves the deadline.
    async fn rotate_refresh_token(&self, live_hash: &str, replacement: &Session) -> Result<bool>;

    /// Delete one live session by hash. Idempotent: deleting an unknown or
    /// already-removed hash succeeds without effect. Does not touch retirement
    /// records.
    async fn revoke_session(&self, token_hash: &str) -> Result<()>;

    /// Remove the family's live generation and every retained retirement
    /// record for `family_id`, returning the total number removed (SR5).
    /// Idempotent: revoking an unknown (but well-formed) family id returns
    /// `Ok(0)`.
    async fn revoke_family(&self, family_id: &str) -> Result<u64>;

    /// Remove every live generation and retained retirement record across all
    /// of a user's families (the SR5 guarantee, family-set-wide). Idempotent.
    async fn revoke_all_user_sessions(&self, user_id: &str) -> Result<()>;

    /// Count live sessions whose `expires_at` is in the future.
    async fn count_active_sessions(&self) -> Result<u64>;

    /// Delete all expired rows — sessions past their `expires_at` and
    /// retirement records past their retention deadline alike — returning the
    /// combined number deleted.
    async fn cleanup_expired_sessions(&self) -> Result<u64>;
}
