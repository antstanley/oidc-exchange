use async_trait::async_trait;
use oidc_exchange_core::domain::{AuditEvent, RateLimitDecision, RateLimitKey, User};
use oidc_exchange_core::error::Error;
use oidc_exchange_core::error::Result;
use oidc_exchange_core::ports::{AuditLog, KeyManager, RateLimiter, UserSync};

/// A no-op audit log that silently discards all events.
///
/// Used as the default when `audit.adapter = "noop"`.
pub struct NoopAuditLog;

impl NoopAuditLog {
    pub fn new() -> Self {
        Self
    }
}

impl Default for NoopAuditLog {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AuditLog for NoopAuditLog {
    async fn emit(&self, _event: &AuditEvent) -> Result<()> {
        Ok(())
    }
}

/// A no-op rate limiter that always permits requests without retaining state.
#[derive(Default)]
pub struct NoopRateLimiter;

impl NoopRateLimiter {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl RateLimiter for NoopRateLimiter {
    async fn check_and_consume(&self, _key: &RateLimitKey) -> Result<RateLimitDecision> {
        Ok(RateLimitDecision::Allow)
    }
}

/// A no-op key manager that panics if used for signing.
///
/// Used when the server runs in `admin` role and doesn't need JWT signing.
pub struct NoopKeyManager;

#[async_trait]
impl KeyManager for NoopKeyManager {
    async fn sign(&self, _payload: &[u8]) -> Result<Vec<u8>> {
        Err(Error::KeyError {
            detail: "NoopKeyManager: signing not available in admin-only mode".into(),
        })
    }

    async fn verify(&self, _payload: &[u8], _signature: &[u8]) -> Result<bool> {
        Err(Error::KeyError {
            detail: "NoopKeyManager: verification not available in admin-only mode".into(),
        })
    }

    async fn public_jwk(&self) -> Result<serde_json::Value> {
        Err(Error::KeyError {
            detail: "NoopKeyManager: JWKS not available in admin-only mode".into(),
        })
    }

    fn algorithm(&self) -> &str {
        "none"
    }

    fn key_id(&self) -> &str {
        "noop"
    }
}

/// A no-op user sync adapter that does nothing on user lifecycle events.
///
/// Used when `user_sync.enabled = false`.
pub struct NoopUserSync;

impl NoopUserSync {
    pub fn new() -> Self {
        Self
    }
}

impl Default for NoopUserSync {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl UserSync for NoopUserSync {
    async fn notify_user_created(&self, _user: &User) -> Result<()> {
        Ok(())
    }

    async fn notify_user_updated(&self, _user: &User, _changed_fields: &[&str]) -> Result<()> {
        Ok(())
    }

    async fn notify_user_deleted(&self, _user_id: &str) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};

    use super::NoopRateLimiter;
    use oidc_exchange_core::domain::{RateLimitDecision, RateLimitKey};
    use oidc_exchange_core::ports::RateLimiter;

    #[tokio::test]
    async fn noop_rate_limiter_always_allows_without_state() {
        let limiter = NoopRateLimiter::new();
        let key = RateLimitKey::ClientAddr(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 5)));
        assert_eq!(
            limiter.check_and_consume(&key).await.unwrap(),
            RateLimitDecision::Allow
        );
        assert_eq!(
            limiter.check_and_consume(&key).await.unwrap(),
            RateLimitDecision::Allow
        );
    }
}
