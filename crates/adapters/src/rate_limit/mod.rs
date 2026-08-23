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
/// nothing — they never reach [`RateLimiter::consume`]. Phase two (lockout):
/// once the budget is exhausted the peer is denied outright for the remainder
/// of `auth_failure_window + auth_lockout`, with `retry_after_secs` reporting
/// the remaining denial time.
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

    /// Run `f` against the lock-protected state. A poisoned lock means a prior
    /// update panicked mid-mutation; failing closed here would lock out every
    /// operator permanently, while failing open leaves throttling degraded but
    /// audited — so every operation degrades to "unthrottled" and says so.
    fn with_state<T>(&self, f: impl FnOnce(&mut HashMap<IpAddr, Bucket>) -> T) -> Option<T> {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(_) => {
                tracing::error!(
                    "admin auth rate-limiter lock poisoned; allowing request unthrottled"
                );
                return None;
            }
        };
        Some(f(&mut state))
    }

    /// Expire finished buckets so stale peers cannot crowd the bound, then get
    /// (creating if needed) the bucket for `peer` at instant `now`.
    fn live_bucket<'a>(
        &self,
        state: &'a mut HashMap<IpAddr, Bucket>,
        peer: &IpAddr,
        now: Instant,
    ) -> &'a mut Bucket {
        let lifetime = self.bucket_lifetime();
        state.retain(|_, bucket| now.duration_since(bucket.started_at) < lifetime);

        if !state.contains_key(peer) {
            let fresh = Bucket {
                started_at: now,
                consumed: 0,
            };
            state.insert(*peer, fresh);
            // Bound enforcement: after expiry-retention above, inserting one
            // more entry can exceed the cap by at most one; evict the oldest
            // live bucket to make room.
            if state.len() > MAX_TRACKED_PEERS {
                let oldest = state
                    .iter()
                    .min_by_key(|(_, bucket)| bucket.started_at)
                    .map(|(addr, _)| *addr)
                    .expect("a non-empty map always has an oldest entry");
                assert_ne!(
                    oldest, *peer,
                    "the freshly inserted bucket must never be its own eviction victim"
                );
                state.remove(&oldest);
            }
        }

        // Capture the bound check before `get_mut`: the bucket borrow must
        // stay live for the return value, and a second (immutable) borrow of
        // `state` after `get_mut` would alias it.
        let tracked_peers = state.len();
        let bucket = state.get_mut(peer).expect("bucket was just inserted");
        debug_assert!(
            tracked_peers <= MAX_TRACKED_PEERS,
            "peer tracking must stay bounded by MAX_TRACKED_PEERS"
        );
        bucket
    }

    /// The consultation half of the contract: locked out or not, consuming
    /// nothing either way.
    fn check_at(&self, peer: &IpAddr, now: Instant) -> RateLimitDecision {
        self.with_state(|state| {
            let bucket = self.live_bucket(state, peer, now);
            Self::decision_at(bucket, now, self.bucket_lifetime(), self.max_auth_failures)
        })
        .unwrap_or(RateLimitDecision::Allow)
    }

    /// The consumption half: record one failed attempt, tripping lockout when
    /// the last free unit goes.
    fn consume_at(&self, peer: &IpAddr, now: Instant) -> RateLimitDecision {
        self.with_state(|state| {
            let lifetime = self.bucket_lifetime();
            let bucket = self.live_bucket(state, peer, now);
            let elapsed = now.duration_since(bucket.started_at);

            // A fully elapsed phase — or one whose counting window closed
            // without ever exhausting the budget — rolls over here: the aged
            // failures stop counting, and this attempt is recorded as the new
            // phase's first unit below.
            if elapsed >= lifetime
                || (elapsed >= self.failure_window && bucket.consumed < self.max_auth_failures)
            {
                *bucket = Bucket {
                    started_at: now,
                    consumed: 0,
                };
            }

            if bucket.consumed < self.max_auth_failures {
                bucket.consumed += 1;
                if bucket.consumed >= self.max_auth_failures {
                    // This consumption just spent the last free unit: the
                    // lockout trips and runs from the phase start until
                    // window + lockout have fully elapsed.
                    return RateLimitDecision::Deny {
                        retry_after_secs: remaining_secs(lifetime, elapsed),
                    };
                }
                return RateLimitDecision::Allow;
            }

            // Already locked out: earlier failures inside this phase's
            // lifetime spent the budget.
            RateLimitDecision::Deny {
                retry_after_secs: remaining_secs(lifetime, elapsed),
            }
        })
        .unwrap_or(RateLimitDecision::Allow)
    }

    /// Locked out or not, for a bucket observed at `now` (no mutation).
    ///
    /// Allowed while the counting phase still has units left; denied once the
    /// budget is spent — including inside the counting window, so the lockout
    /// is flat from the trip instant until window + lockout have fully run.
    /// A phase whose window closed without exhaustion is stale-but-harmless:
    /// it still answers Allow, and the next consumption rolls it.
    fn decision_at(
        bucket: &Bucket,
        now: Instant,
        lifetime: Duration,
        max_auth_failures: u64,
    ) -> RateLimitDecision {
        let elapsed = now.duration_since(bucket.started_at);
        if bucket.consumed < max_auth_failures || elapsed >= lifetime {
            RateLimitDecision::Allow
        } else {
            RateLimitDecision::Deny {
                retry_after_secs: remaining_secs(lifetime, elapsed),
            }
        }
    }
}

#[async_trait]
impl RateLimiter for AdminAuthRateLimiter {
    async fn check(&self, key: &RateLimitKey) -> Result<RateLimitDecision> {
        match key {
            // This adapter implements exactly the admin plane's budget; other
            // key scopes are #24's concern and are rejected loudly rather than
            // silently allowed into the wrong budget.
            RateLimitKey::OperatorAuth(peer) => Ok(self.check_at(peer, Instant::now())),
        }
    }

    async fn consume(&self, key: &RateLimitKey) -> Result<RateLimitDecision> {
        match key {
            RateLimitKey::OperatorAuth(peer) => Ok(self.consume_at(peer, Instant::now())),
        }
    }
}

/// Seconds of `lifetime` still ahead at `elapsed`, floored at one so a
/// boundary hit never advertises an immediate retry.
fn remaining_secs(lifetime: Duration, elapsed: Duration) -> u64 {
    lifetime.saturating_sub(elapsed).as_secs().max(1)
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
    async fn denies_from_the_failure_that_spends_the_budget() {
        let limiter = limiter(2);
        let addr = peer(1);
        let k = key(addr);

        // Consultations alone never draw the budget down — only consume does.
        for _ in 0..5 {
            assert_eq!(limiter.check(&k).await.unwrap(), RateLimitDecision::Allow);
        }

        assert_eq!(limiter.consume(&k).await.unwrap(), RateLimitDecision::Allow);
        // Second failure spends the last free unit and trips the lockout...
        assert!(matches!(
            limiter.consume(&k).await.unwrap(),
            RateLimitDecision::Deny { retry_after_secs } if retry_after_secs >= 1
        ));
        // ...every later consumption is denied too, and from then on even
        // consultations are denied (the lockout is flat, not window-scoped).
        assert!(matches!(
            limiter.consume(&k).await.unwrap(),
            RateLimitDecision::Deny { .. }
        ));
        assert!(matches!(
            limiter.check(&k).await.unwrap(),
            RateLimitDecision::Deny { .. }
        ));
    }

    #[tokio::test]
    async fn budgets_are_per_peer_and_lockout_outlives_the_counting_window() {
        let limiter = limiter(1);
        let a = peer(2);
        let b = peer(3);

        assert_eq!(
            limiter.check(&key(a)).await.unwrap(),
            RateLimitDecision::Allow
        );
        // Peer b has its own bucket: no cross-peer draw-down.
        assert_eq!(
            limiter.check(&key(b)).await.unwrap(),
            RateLimitDecision::Allow
        );
        // Peer a fails once — its whole budget (1) — and trips immediately.
        assert!(matches!(
            limiter.consume(&key(a)).await.unwrap(),
            RateLimitDecision::Deny { .. }
        ));
        // Peer a stays locked out for consultations, but that lockout never
        // leaks onto b, whose budget is untouched.
        assert!(matches!(
            limiter.check(&key(a)).await.unwrap(),
            RateLimitDecision::Deny { .. }
        ));
        assert_eq!(
            limiter.check(&key(b)).await.unwrap(),
            RateLimitDecision::Allow
        );
        // Peer b's own first failure then trips its own budget.
        assert!(matches!(
            limiter.consume(&key(b)).await.unwrap(),
            RateLimitDecision::Deny { .. }
        ));
        assert!(matches!(
            limiter.check(&key(b)).await.unwrap(),
            RateLimitDecision::Deny { .. }
        ));
    }

    /// A phase whose counting window closed without exhausting the budget
    /// rolls over: the aged failures stop counting and the next consumption
    /// starts a fresh phase instead of being denied by stale state.
    #[tokio::test]
    async fn unexhausted_phases_roll_over_after_the_window_closes() {
        let window_secs = 1u64;
        let limiter =
            AdminAuthRateLimiter::new(3, Duration::from_secs(window_secs), Duration::from_secs(1))
                .expect("valid throttle bounds");
        let k = key(peer(4));

        // Two failures inside the window: the budget (3) is not exhausted.
        assert_eq!(limiter.consume(&k).await.unwrap(), RateLimitDecision::Allow);
        assert_eq!(limiter.consume(&k).await.unwrap(), RateLimitDecision::Allow);
        assert_eq!(limiter.check(&k).await.unwrap(), RateLimitDecision::Allow);

        // Let the counting window close; the stale phase must still answer
        // Allow, never a stale denial.
        tokio::time::sleep(Duration::from_millis(window_secs * 1000 + 50)).await;
        assert_eq!(limiter.check(&k).await.unwrap(), RateLimitDecision::Allow);

        // The next consumption rolls the phase: two more failures are needed
        // before the fresh budget trips, proving the old ones stopped counting.
        assert_eq!(limiter.consume(&k).await.unwrap(), RateLimitDecision::Allow);
        assert_eq!(limiter.consume(&k).await.unwrap(), RateLimitDecision::Allow);
        assert!(matches!(
            limiter.consume(&k).await.unwrap(),
            RateLimitDecision::Deny { .. }
        ));
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
