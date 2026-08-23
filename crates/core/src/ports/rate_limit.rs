//! VENDORED SEAM (task 03): the rate-limit port, modelled on sibling PR #24
//! (`2026-08-05-audit_and_throttle_authentication_failures`, branch
//! `spec/audit-and-throttle-auth-failures`). This branch carries it so the
//! admin plane can throttle failed operator authentications; at merge time
//! this file is deleted in favour of #24's identical port.
//!
//! One deliberate divergence from a single `check_and_consume` shape: the
//! source spec requires that "a unit is consumed only by a failed attempt",
//! so the auth layer must consult the budget *before* evaluating a credential
//! without drawing it down. Expressing consult-without-consume and
//! consume-on-failure needs the two methods below; if #24's port lands as one
//! method, this seam's call sites adapt to it then.

use async_trait::async_trait;

use crate::domain::{RateLimitDecision, RateLimitKey};
use crate::error::Result;

/// A bounded rate-limit budget over [`RateLimitKey`]s.
#[async_trait]
pub trait RateLimiter: Send + Sync {
    /// Consult whether `key` is currently locked out, consuming nothing.
    /// `Deny { retry_after_secs }` means locked out for at least that many
    /// seconds; `Allow` means the key may proceed to whatever follows the
    /// consultation (credential evaluation, request handling).
    async fn check(&self, key: &RateLimitKey) -> Result<RateLimitDecision>;

    /// Consume one unit from `key`'s budget — called only when an attempt
    /// *fails* — returning `Deny` once the consumption trips lockout.
    async fn consume(&self, key: &RateLimitKey) -> Result<RateLimitDecision>;
}
