use async_trait::async_trait;

use crate::domain::{RateLimitDecision, RateLimitKey};
use crate::error::Result;

/// Consumes one unit from a bounded rate-limit bucket without prescribing storage or I/O.
#[async_trait]
pub trait RateLimiter: Send + Sync {
    async fn check_and_consume(&self, key: &RateLimitKey) -> Result<RateLimitDecision>;
}
