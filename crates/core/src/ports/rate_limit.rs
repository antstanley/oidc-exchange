//! VENDORED SEAM (task 03): the rate-limit port, vendored verbatim from
//! sibling PR #24 (`2026-08-05-audit_and_throttle_authentication_failures`,
//! branch `spec/audit-and-throttle-auth-failures`). This branch carries it so
//! the admin plane can throttle failed operator authentications; at merge time
//! this file is deleted in favour of #24's identical port.

use async_trait::async_trait;

use crate::domain::{RateLimitDecision, RateLimitKey};
use crate::error::Result;

/// Consumes one unit from a bounded rate-limit bucket without prescribing storage or I/O.
#[async_trait]
pub trait RateLimiter: Send + Sync {
    async fn check_and_consume(&self, key: &RateLimitKey) -> Result<RateLimitDecision>;
}
