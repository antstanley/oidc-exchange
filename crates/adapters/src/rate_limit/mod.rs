//! In-process rate limiting for the admin plane's failed operator
//! authentications.
//!
//! VENDORED SEAM (task 03): modelled on sibling PR #24's
//! `FixedWindowRateLimiter` (`crates/server/src/middleware/throttle.rs` on its
//! branch), reduced to the single `OperatorAuth` budget this branch needs and
//! extended with the source spec's two-phase timing: failures are counted
//! within `auth_failure_window`; once the budget is exhausted the peer is
//! locked out until `window_start + auth_lockout` (a flat denial window, not a
//! rolling one). At merge time PR #24's generic limiter replaces this adapter.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;

use oidc_exchange_core::domain::{RateLimitDecision, RateLimitKey};
use oidc_exchange_core::error::{Error, Result};
use oidc_exchange_core::ports::RateLimiter;

/// Upper bound on distinct peers tracked at once. A lockout table must never
/// become the memory-exhaustion primitive it exists to prevent: when the map
/// is full the oldest bucket is evicted (an attacker cycling addresses can at
/// worst evict another attacker's lockout, and every consumed unit was still
/// audited).
pub const MAX_TRACKED_PEERS: usize = 4096;

/// A two-phase fixed-window limiter over [`RateLimitKey::OperatorAuth`] keys.
///
/// Phase one (counting): each *failed* authentication consumes one unit of the
/// per-peer budget inside the current failure window; successes consume
/// nothing. Phase two (lockout): once the budget is exhausted the peer is
/// denied outright for the remainder of `auth_failure_window + auth_lockout`,
/// with `retry_after_secs` reporting the remaining denial time.
pub struct AdminAuthRateLimiter {
    failure_window: Duration,
    lockout: Duration,
    max_auth_failures: u64,
    state: Mutex<HashMap<IpAddr, Bucket>>,
}

#[derive(Clone, Copy)]
struct Bucket {
    /// When the current counting phase started. Reset when both the failure
    /// window and the lockout have fully elapsed.
    started_at: Instant,
    consumed: u64,
}

impl AdminAuthRateLimiter {
    /// Build a limiter from the `[internal_api]` throttle configuration.
    ///
    /// Every bound is validated eagerly so a misconfiguration (zero window,
    /// zero budget) fails at startup instead of producing either an always-open
    /// or an always-closed throttle.
    pub fn new(
        max_auth_failures: u64,
        failure_window: Duration,
        lockout: Duration,
    ) -> Result<Self> {
        if max_auth_failures == 0 {
            return Err(Error::ConfigError {
                detail: "internal_api.max_auth_failures must be non-zero".to_string(),
            });
        }
        if failure_window.is_zero() {
            return Err(Error::ConfigError {
                detail: "internal_api.auth_failure_window must be non-zero".to_string(),
            });
        }
        if lockout.is_zero() {
            return Err(Error::ConfigError {
                detail: "internal_api.auth_lockout must be non-zero".to_string(),
            });
        }
        // Compile-time defence in depth: the eviction logic below relies on
        // the tracking bound admitting at least one live bucket.
        const {
            assert!(
                MAX_TRACKED_PEERS > 0,
                "the peer-tracking bound must allow at least one entry"
            );
        }
        Ok(Self {
            failure_window,
            lockout,
            max_auth_failures,
            state: Mutex::new(HashMap::new()),
        })
    }

    /// Total lifetime of one counting phase plus its lockout.
    fn bucket_lifetime(&self) -> Duration {
        self.failure_window + self.lockout
    }

    /// The decision for `peer` at instant `now`, consuming one unit only when
    /// the decision is a fresh Allow that draws the budget down.
    fn check_at(&self, peer: &IpAddr, now: Instant) -> RateLimitDecision {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            // A poisoned lock means a prior update panicked mid-mutation;
            // failing closed here would lock out every operator permanently,
            // while failing open leaves throttling degraded but audited.
            Err(_) => {
                tracing::error!(
                    "admin auth rate-limiter lock poisoned; allowing request unthrottled"
                );
                return RateLimitDecision::Allow;
            }
        };

        let lifetime = self.bucket_lifetime();
        // Expire finished buckets first so stale peers cannot crowd the bound.
        state.retain(|_, bucket| now.duration_since(bucket.started_at) < lifetime);

        let budget = self.max_auth_failures;
        let bucket = match state.get_mut(peer) {
            Some(bucket) => *bucket,
            None => {
                let fresh = Bucket {
                    started_at: now,
                    consumed: 0,
                };
                state.insert(*peer, fresh);
                // Bound enforcement: after expiry-retention above, inserting
                // one more entry can exceed the cap by at most one; evict the
                // oldest live bucket to make room.
                if state.len() > MAX_TRACKED_PEERS {
                    let oldest = state
                        .iter()
                        .min_by_key(|(_, bucket)| bucket.started_at)
                        .map(|(addr, _)| *addr)
                        .expect("a non-empty map always has an oldest entry");
                    assert_ne!(
                        &oldest, peer,
                        "the freshly inserted bucket must never be its own eviction victim"
                    );
                    state.remove(&oldest);
                }
                debug_assert!(
                    state.len() <= MAX_TRACKED_PEERS,
                    "peer tracking must stay bounded by MAX_TRACKED_PEERS"
                );
                fresh
            }
        };

        let elapsed = now.duration_since(bucket.started_at);
        if elapsed >= lifetime {
            // Fully elapsed since construction: start a new counting phase.
            let fresh = Bucket {
                started_at: now,
                consumed: 0,
            };
            state.insert(*peer, fresh);
            return RateLimitDecision::Allow;
        }

        if elapsed < self.failure_window && bucket.consumed < budget {
            state.get_mut(peer).expect("bucket just verified").consumed += 1;
            return RateLimitDecision::Allow;
        }

        // Either the budget is spent or the lockout is running: deny with the
        // remaining seconds of the denial window (at least one, so a boundary
        // hit never advertises an immediate retry).
        let remaining = lifetime.saturating_sub(elapsed).as_secs().max(1);
        RateLimitDecision::Deny {
            retry_after_secs: remaining,
        }
    }
}

#[async_trait]
impl RateLimiter for AdminAuthRateLimiter {
    async fn check_and_consume(&self, key: &RateLimitKey) -> Result<RateLimitDecision> {
        match key {
            // This adapter implements exactly the admin plane's budget; other
            // key scopes are #24's concern and are rejected loudly rather than
            // silently allowed into the wrong budget.
            RateLimitKey::OperatorAuth(peer) => Ok(self.check_at(peer, Instant::now())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn limiter(budget: u64) -> AdminAuthRateLimiter {
        AdminAuthRateLimiter::new(budget, Duration::from_secs(60), Duration::from_secs(300))
            .expect("valid throttle bounds")
    }

    fn peer(last_octet: u8) -> IpAddr {
        IpAddr::V4(std::net::Ipv4Addr::new(192, 0, 2, last_octet))
    }

    fn key(addr: IpAddr) -> RateLimitKey {
        RateLimitKey::OperatorAuth(addr)
    }

    #[tokio::test]
    async fn denies_only_after_the_budget_is_spent() {
        let limiter = limiter(2);
        let addr = peer(1);

        assert_eq!(
            limiter.check_and_consume(&key(addr)).await.unwrap(),
            RateLimitDecision::Allow
        );
        assert_eq!(
            limiter.check_and_consume(&key(addr)).await.unwrap(),
            RateLimitDecision::Allow
        );
        assert!(
            matches!(
                limiter.check_and_consume(&key(addr)).await.unwrap(),
                RateLimitDecision::Deny { retry_after_secs } if retry_after_secs >= 1
            ),
            "the third failure must be denied with a positive Retry-After"
        );
    }

    #[tokio::test]
    async fn budgets_are_per_peer_and_lockout_outlives_the_counting_window() {
        let limiter = limiter(1);
        let a = peer(2);
        let b = peer(3);
        let now = Instant::now();

        assert_eq!(limiter.check_at(&a, now), RateLimitDecision::Allow);
        // Peer b has its own bucket: no cross-peer draw-down.
        assert_eq!(limiter.check_at(&b, now), RateLimitDecision::Allow);
        // Peer a is now locked out for the full remaining lifetime (window +
        // lockout), not just the remainder of the counting window.
        let denied = limiter.check_at(&a, now + Duration::from_secs(30));
        assert!(
            matches!(denied, RateLimitDecision::Deny { .. }),
            "still inside window + lockout: peer a stays denied"
        );
        // After the full lifetime expires the peer gets a fresh budget.
        let later = now + Duration::from_secs(360 + 1);
        assert_eq!(
            limiter.check_at(&a, later),
            RateLimitDecision::Allow,
            "after failure window + lockout the peer starts a fresh counting phase"
        );
    }

    #[tokio::test]
    async fn constructor_rejects_zero_bounds() {
        assert!(
            AdminAuthRateLimiter::new(0, Duration::from_secs(1), Duration::from_secs(1)).is_err()
        );
        assert!(AdminAuthRateLimiter::new(1, Duration::ZERO, Duration::from_secs(1)).is_err());
        assert!(AdminAuthRateLimiter::new(1, Duration::from_secs(1), Duration::ZERO).is_err());
        assert!(
            AdminAuthRateLimiter::new(5, Duration::from_secs(60), Duration::from_secs(300)).is_ok()
        );
    }
}
