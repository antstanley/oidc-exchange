//! The rate-limit port: bounded budgets over [`RateLimitKey`]s.
//!
//! Two consumption shapes coexist because two planes need them:
//!
//! - The exchange plane throttles *requests*: every attempt draws down the
//!   budget, so [`RateLimiter::check_and_consume`] decides and consumes in
//!   one step.
//! - The admin plane throttles *failed authentications only* (the source
//!   spec requires that "a unit is consumed only by a failed attempt"), so
//!   its callers consult [`RateLimiter::check`] before evaluating a
//!   credential — consuming nothing — and call [`RateLimiter::consume`]
//!   only when the attempt fails.

use async_trait::async_trait;

use crate::domain::{RateLimitDecision, RateLimitKey};
use crate::error::Result;

/// A bounded rate-limit budget over [`RateLimitKey`]s.
#[async_trait]
pub trait RateLimiter: Send + Sync {
    /// Decide and consume in one step: every call draws down `key`'s budget.
    async fn check_and_consume(&self, key: &RateLimitKey) -> Result<RateLimitDecision>;

    /// Consult whether `key` is currently locked out, consuming nothing.
    /// `Deny { retry_after_secs }` means locked out for at least that many
    /// seconds; `Allow` means the key may proceed to whatever follows the
    /// consultation (credential evaluation, request handling).
    async fn check(&self, key: &RateLimitKey) -> Result<RateLimitDecision>;

    /// Consume one unit from `key`'s budget — called only when an attempt
    /// *fails* — returning `Deny` once the consumption trips lockout.
    async fn consume(&self, key: &RateLimitKey) -> Result<RateLimitDecision>;
}
