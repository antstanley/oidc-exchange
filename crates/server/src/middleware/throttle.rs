use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use oidc_exchange_core::domain::{RateLimitDecision, RateLimitKey};
use oidc_exchange_core::error::{Error, Result};
use oidc_exchange_core::ports::RateLimiter;

/// In-process fixed-window limiter. State is bounded by `max_entries`; expired
/// buckets are evicted before every insertion and the oldest live bucket is
/// evicted when capacity is full.
pub struct FixedWindowRateLimiter {
    window: Duration,
    budgets: RateLimitBudgets,
    max_entries: usize,
    clock: Arc<dyn Clock>,
    state: Mutex<HashMap<RateLimitKey, Bucket>>,
}

pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
}

struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

#[derive(Clone)]
pub struct TestClock {
    now: Arc<RwLock<Instant>>,
}

impl TestClock {
    pub fn new(now: Instant) -> Self {
        Self {
            now: Arc::new(RwLock::new(now)),
        }
    }

    pub fn advance(&self, duration: Duration) {
        *self.now.write().expect("test clock lock is not poisoned") += duration;
    }
}

impl Clock for TestClock {
    fn now(&self) -> Instant {
        *self.now.read().expect("test clock lock is not poisoned")
    }
}

/// Fixed-window budgets selected by the rate-limit key's scope. A zero budget
/// explicitly disables only that scope.
#[derive(Debug, Clone, Copy)]
pub struct RateLimitBudgets {
    pub per_ip: u64,
    pub per_ip_failures: u64,
    pub per_subject: u64,
    pub per_provider: u64,
}

impl RateLimitBudgets {
    fn for_key(&self, key: &RateLimitKey) -> u64 {
        match key {
            RateLimitKey::ClientAddr(_) => self.per_ip,
            RateLimitKey::ClientAddrFailure(_) => self.per_ip_failures,
            RateLimitKey::Subject { .. } => self.per_subject,
            RateLimitKey::Provider(_) => self.per_provider,
            // The admin plane's budget lives in AdminAuthRateLimiter; routing
            // its key into the exchange window is a wiring bug.
            RateLimitKey::OperatorAuth(_) => unreachable!(
                "the OperatorAuth budget belongs to AdminAuthRateLimiter, not the fixed window"
            ),
        }
    }
}

#[derive(Clone, Copy)]
struct Bucket {
    started_at: Instant,
    consumed: u64,
}

impl FixedWindowRateLimiter {
    pub fn new(window: Duration, budgets: RateLimitBudgets, max_entries: usize) -> Result<Self> {
        Self::with_clock(window, budgets, max_entries, Arc::new(SystemClock))
    }

    pub fn with_clock(
        window: Duration,
        budgets: RateLimitBudgets,
        max_entries: usize,
        clock: Arc<dyn Clock>,
    ) -> Result<Self> {
        if window.is_zero() {
            return Err(Error::ConfigError {
                detail: "rate limit window must be non-zero".to_string(),
            });
        }
        if max_entries == 0 {
            return Err(Error::ConfigError {
                detail: "rate limit max_entries must be non-zero".to_string(),
            });
        }
        Ok(Self {
            window,
            budgets,
            max_entries,
            clock,
            state: Mutex::new(HashMap::new()),
        })
    }

    /// Check against an explicitly supplied instant. This is intentionally public so router
    /// integration tests can exercise the real limiter deterministically without sleeping.
    pub fn check_at(&self, key: &RateLimitKey, now: Instant) -> Result<RateLimitDecision> {
        let budget = self.budgets.for_key(key);
        if budget == 0 {
            return Ok(RateLimitDecision::Allow);
        }

        let mut state = self.state.lock().map_err(|_| Error::ConfigError {
            detail: "in-process rate limiter state lock is poisoned".to_string(),
        })?;
        state.retain(|_, bucket| now.duration_since(bucket.started_at) < self.window);

        if !state.contains_key(key) && state.len() == self.max_entries {
            let oldest = state
                .iter()
                .min_by_key(|(_, bucket)| bucket.started_at)
                .map(|(key, _)| key.clone())
                .expect("a full non-empty rate-limit map has an oldest entry");
            state.remove(&oldest);
        }

        let bucket = state.entry(key.clone()).or_insert(Bucket {
            started_at: now,
            consumed: 0,
        });
        if bucket.consumed < budget {
            bucket.consumed += 1;
            return Ok(RateLimitDecision::Allow);
        }
        let elapsed = now.duration_since(bucket.started_at);
        let remaining = self.window.saturating_sub(elapsed).as_secs().max(1);
        Ok(RateLimitDecision::Deny {
            retry_after_secs: remaining,
        })
    }

    #[cfg(test)]
    fn entry_count(&self) -> usize {
        self.state.lock().unwrap().len()
    }
}

#[async_trait::async_trait]
impl RateLimiter for FixedWindowRateLimiter {
    async fn check_and_consume(&self, key: &RateLimitKey) -> Result<RateLimitDecision> {
        self.check_at(key, self.clock.now())
    }

    // The exchange-plane fixed window has no consult-without-consume shape:
    // its budgets meter requests, not failed attempts. The admin plane uses
    // `AdminAuthRateLimiter`; routing an operator-auth key here is a wiring
    // bug, surfaced loudly.
    async fn check(&self, key: &RateLimitKey) -> Result<RateLimitDecision> {
        unreachable!(
            "FixedWindowRateLimiter meters requests via check_and_consume; \
             consult-only check is not part of its contract (key {key:?})"
        )
    }

    async fn consume(&self, key: &RateLimitKey) -> Result<RateLimitDecision> {
        self.check_and_consume(key).await
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};
    use std::time::{Duration, Instant};

    use super::{FixedWindowRateLimiter, RateLimitBudgets};
    use oidc_exchange_core::domain::{RateLimitDecision, RateLimitKey};

    fn budgets(budget: u64) -> RateLimitBudgets {
        RateLimitBudgets {
            per_ip: budget,
            per_ip_failures: budget,
            per_subject: budget,
            per_provider: budget,
        }
    }

    fn key(last_octet: u8) -> RateLimitKey {
        RateLimitKey::ClientAddr(IpAddr::V4(Ipv4Addr::new(192, 0, 2, last_octet)))
    }

    #[test]
    fn fixed_window_denies_after_budget_and_expires() {
        let limiter = FixedWindowRateLimiter::new(Duration::from_secs(60), budgets(2), 4).unwrap();
        let now = Instant::now();
        assert_eq!(
            limiter.check_at(&key(1), now).unwrap(),
            RateLimitDecision::Allow
        );
        assert_eq!(
            limiter.check_at(&key(1), now).unwrap(),
            RateLimitDecision::Allow
        );
        assert!(matches!(
            limiter.check_at(&key(1), now).unwrap(),
            RateLimitDecision::Deny { .. }
        ));
        assert_eq!(
            limiter
                .check_at(&key(1), now + Duration::from_secs(60))
                .unwrap(),
            RateLimitDecision::Allow
        );
    }

    #[test]
    fn expired_entries_are_evicted_and_live_entries_are_bounded() {
        let limiter = FixedWindowRateLimiter::new(Duration::from_secs(60), budgets(1), 2).unwrap();
        let now = Instant::now();
        limiter.check_at(&key(1), now).unwrap();
        limiter.check_at(&key(2), now).unwrap();
        limiter.check_at(&key(3), now).unwrap();
        assert_eq!(limiter.entry_count(), 2);
        limiter
            .check_at(&key(4), now + Duration::from_secs(60))
            .unwrap();
        assert_eq!(limiter.entry_count(), 1);
    }

    #[test]
    fn distinct_scope_budgets_are_selected_by_key() {
        let limiter = FixedWindowRateLimiter::new(
            Duration::from_secs(60),
            RateLimitBudgets {
                per_ip: 1,
                per_ip_failures: 2,
                per_subject: 3,
                per_provider: 4,
            },
            8,
        )
        .unwrap();
        let now = Instant::now();
        let keys = [
            (key(1), 1),
            (
                RateLimitKey::ClientAddrFailure(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 2))),
                2,
            ),
            (RateLimitKey::subject(None, "subject").unwrap(), 3),
            (RateLimitKey::provider("idp").unwrap(), 4),
        ];

        for (key, allowed) in keys {
            for _ in 0..allowed {
                assert_eq!(
                    limiter.check_at(&key, now).unwrap(),
                    RateLimitDecision::Allow
                );
            }
            assert!(matches!(
                limiter.check_at(&key, now).unwrap(),
                RateLimitDecision::Deny { .. }
            ));
        }
    }

    #[test]
    fn zero_scope_budget_disables_only_that_scope() {
        let limiter = FixedWindowRateLimiter::new(
            Duration::from_secs(60),
            RateLimitBudgets {
                per_ip: 1,
                per_ip_failures: 0,
                per_subject: 1,
                per_provider: 1,
            },
            4,
        )
        .unwrap();
        let now = Instant::now();
        let failure_key = RateLimitKey::ClientAddrFailure(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)));
        for _ in 0..3 {
            assert_eq!(
                limiter.check_at(&failure_key, now).unwrap(),
                RateLimitDecision::Allow
            );
        }
        assert_eq!(limiter.entry_count(), 0);
        assert_eq!(
            limiter.check_at(&key(1), now).unwrap(),
            RateLimitDecision::Allow
        );
        assert!(matches!(
            limiter.check_at(&key(1), now).unwrap(),
            RateLimitDecision::Deny { .. }
        ));
    }

    #[test]
    fn client_address_and_failure_scopes_are_independent() {
        let limiter = FixedWindowRateLimiter::new(
            Duration::from_secs(60),
            RateLimitBudgets {
                per_ip: 1,
                per_ip_failures: 1,
                per_subject: 1,
                per_provider: 1,
            },
            4,
        )
        .unwrap();
        let now = Instant::now();
        let address = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1));
        let request_key = RateLimitKey::ClientAddr(address);
        let failure_key = RateLimitKey::ClientAddrFailure(address);

        assert_eq!(
            limiter.check_at(&request_key, now).unwrap(),
            RateLimitDecision::Allow
        );
        assert_eq!(
            limiter.check_at(&failure_key, now).unwrap(),
            RateLimitDecision::Allow
        );
        assert!(matches!(
            limiter.check_at(&request_key, now).unwrap(),
            RateLimitDecision::Deny { .. }
        ));
        assert!(matches!(
            limiter.check_at(&failure_key, now).unwrap(),
            RateLimitDecision::Deny { .. }
        ));
    }

    #[test]
    fn constructor_rejects_zero_bounds() {
        assert!(FixedWindowRateLimiter::new(Duration::ZERO, budgets(1), 1).is_err());
        assert!(FixedWindowRateLimiter::new(Duration::from_secs(1), budgets(1), 0).is_err());
    }
}
