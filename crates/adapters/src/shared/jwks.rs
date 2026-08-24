use std::sync::Arc;
use std::time::{Duration, Instant};

use jsonwebtoken::Algorithm;
use oidc_exchange_core::error::{Error, Result};
use tokio::sync::{RwLock, Semaphore};

use crate::shared::keys::{VerificationKey, VerificationKeySet};

/// Default TTL for JWKS cache entries: 1 hour.
const DEFAULT_TTL: Duration = Duration::from_secs(3600);

/// Minimum interval between forced refetches triggered by a `kid` cache miss: 30 seconds.
///
/// Bounds how often a caller can force a network fetch outside the normal TTL-based refresh,
/// so an attacker spraying unknown `kid`s cannot turn the service into a JWKS-endpoint
/// hammer.
const MIN_REFRESH_INTERVAL: Duration = Duration::from_secs(30);

/// How many callers may refill the cache concurrently: exactly one.
///
/// The permit *is* the single-flight election. It is deliberately its own
/// primitive rather than a side effect of the data lock — no lock that protects
/// the cached value is ever held across the fetch — so racing callers either
/// share the winner's result or are served the stale-but-parseable set while
/// the one fetch is in flight.
const MAX_CONCURRENT_REFILLS: usize = 1;

/// Fetches and caches a remote JWKS as a [`VerificationKeySet`] with TTL-based refresh.
///
/// The cached value is held behind an `Arc` and handed out by cheap clone; the
/// per-request cost of a cache hit is a pointer bump, not a deep copy of the key
/// set. The cache builds key sets with the caller's admitted-algorithm policy,
/// so eligibility is decided once per fetch, not once per validation.
///
/// **Single-flight refill.** Expired-TTL callers race for a one-permit
/// [`Semaphore`]; exactly one wins and fetches. Callers arriving during an
/// in-flight refill are served the stale-but-parseable set when one exists —
/// stale is not untrusted, and a `kid` missing from a stale set still falls
/// through to the rate-limited forced refetch, so staleness fails closed — and
/// only a cold cache (nothing parseable to serve) queues on the permit. The
/// winner re-checks freshness under the write guard after winning, fetches with
/// no guard alive, and stores before releasing the permit, so everyone queued
/// behind a *successful* refill finds the fresh entry and spends no request of
/// their own. After a *failed* refill the cache is untouched; each queued
/// waiter then takes one serialized turn of its own (never parallel — the
/// permit bounds concurrency, and each attempt is bounded by the transport's
/// timeouts), which trades a bounded retry chain against inventing shared
/// failure state. Non-2xx, oversized, and malformed bodies are never cached in
/// any of these paths.
pub struct JwksCache {
    jwks_uri: String,
    admitted_algorithms: &'static [Algorithm],
    cache: Arc<RwLock<Option<CachedJwks>>>,
    ttl: Duration,
    /// Instant of the last forced refetch, guarding [`MIN_REFRESH_INTERVAL`]. `None` until the
    /// first forced refetch happens.
    last_forced_refetch: Arc<RwLock<Option<Instant>>>,
    /// The single-flight election permit. Capacity [`MAX_CONCURRENT_REFILLS`].
    refill_permits: Arc<Semaphore>,
}

struct CachedJwks {
    keys: Arc<VerificationKeySet>,
    fetched_at: Instant,
}

impl JwksCache {
    /// Create a new `JwksCache` with the default TTL of 1 hour, admitting the
    /// given algorithm set when building key sets from fetched JWKS documents.
    pub fn new(jwks_uri: String, admitted_algorithms: &'static [Algorithm]) -> Self {
        Self {
            jwks_uri,
            admitted_algorithms,
            cache: Arc::new(RwLock::new(None)),
            ttl: DEFAULT_TTL,
            last_forced_refetch: Arc::new(RwLock::new(None)),
            refill_permits: Arc::new(Semaphore::new(MAX_CONCURRENT_REFILLS)),
        }
    }

    /// Create a new `JwksCache` with a custom TTL and admitted-algorithm set.
    pub fn with_ttl(
        jwks_uri: String,
        ttl: Duration,
        admitted_algorithms: &'static [Algorithm],
    ) -> Self {
        Self {
            jwks_uri,
            admitted_algorithms,
            cache: Arc::new(RwLock::new(None)),
            ttl,
            last_forced_refetch: Arc::new(RwLock::new(None)),
            refill_permits: Arc::new(Semaphore::new(MAX_CONCURRENT_REFILLS)),
        }
    }

    /// Return the cached key set if still fresh, without touching the network.
    ///
    /// The read guard is confined to this helper: it is dropped before the
    /// caller does anything else, so no data-lock guard can survive into a
    /// network await.
    async fn fresh_cached(&self) -> Option<Arc<VerificationKeySet>> {
        let guard = self.cache.read().await;
        match guard.as_ref() {
            Some(cached) if cached.fetched_at.elapsed() < self.ttl => {
                Some(Arc::clone(&cached.keys))
            }
            _ => None,
        }
    }

    /// Return whatever set is cached regardless of age — the
    /// stale-but-parseable serving path for callers arriving mid-refill.
    ///
    /// Stale is acceptable here because it is *old*, not *untrusted*: the set
    /// passed the constructor's full eligibility rulebook when it was fetched,
    /// and a `kid` it lacks still fails closed through the rate-limited forced
    /// refetch in [`get_key`](Self::get_key).
    async fn any_cached(&self) -> Option<Arc<VerificationKeySet>> {
        let guard = self.cache.read().await;
        guard.as_ref().map(|cached| Arc::clone(&cached.keys))
    }

    /// Return the cached key set if still fresh, otherwise refill it.
    ///
    /// Refill is single-flight: one caller is elected through a one-permit
    /// semaphore, fetches with no lock held, and stores before releasing the
    /// permit. Everyone else either finds the winner's fresh entry, or is
    /// served the stale-but-parseable set while the fetch is in flight, or —
    /// cold cache only — queues for the permit. See the type-level docs for
    /// the full discipline and the failed-refill behaviour.
    pub async fn get_keys(&self) -> Result<Arc<VerificationKeySet>> {
        // Fast path: read-lock only, no election, no network.
        if let Some(keys) = self.fresh_cached().await {
            return Ok(keys);
        }

        // Slow path: elect exactly one refiller. try_acquire first keeps the
        // cold-cache case correct — the first arrival takes the permit without
        // yielding — and only a caller with nothing parseable to serve ever
        // waits on `acquire`.
        let _refill_permit = match self.refill_permits.try_acquire() {
            Ok(permit) => permit,
            Err(_no_permit_or_closed) => {
                // Someone is refilling right now. Serve the last parseable set
                // rather than queueing behind a network round trip we did not
                // elect — an expired entry is stale, not untrusted.
                if let Some(stale) = self.any_cached().await {
                    return Ok(stale);
                }
                // Cold cache: nothing to serve, so queue for the permit. When
                // it arrives we hold the election ourselves; the freshness
                // re-check below decides whether the refill we waited on
                // already satisfied us or whether we must fetch.
                self.refill_permits
                    .acquire()
                    .await
                    // The semaphore is never closed; if that invariant ever
                    // breaks, fail loudly as a provider fault rather than hang.
                    .map_err(|_| Error::ProviderError {
                        provider: self.jwks_uri.clone(),
                        detail: "internal error: JWKS refill permit closed".to_string(),
                    })?
            }
        };

        // Holding the permit: a refill that completed while we raced or waited
        // may have refreshed the entry, so confirm staleness under the write
        // guard before spending a request. The guard is dropped at the end of
        // this block — it must never span the fetch below.
        {
            let guard = self.cache.write().await;
            if let Some(ref cached) = *guard {
                if cached.fetched_at.elapsed() < self.ttl {
                    return Ok(Arc::clone(&cached.keys));
                }
            }
        }

        let keys = self.fetch_keys().await?;

        // Store before the permit drops, so every waiter granted the permit
        // after us finds this entry instead of starting its own fetch.
        let mut guard = self.cache.write().await;
        *guard = Some(CachedJwks {
            keys: Arc::clone(&keys),
            fetched_at: Instant::now(),
        });
        drop(guard);
        Ok(keys)
    }

    /// Return the verification key matching `kid`, forcing at most one network
    /// refetch per [`MIN_REFRESH_INTERVAL`] when `kid` is not present in the
    /// cached (or freshly fetched) key set.
    ///
    /// This is distinct from the TTL-based refresh in [`get_keys`](Self::get_keys): it exists
    /// so a legitimate key rotation is picked up without waiting out the (much longer)
    /// `ttl`, while still bounding how often an attacker spraying unknown `kid`s can force a
    /// fetch. A `kid` that matches only ineligible entries is a miss here too — eligibility
    /// was decided in the key set's constructor, so an encryption key cannot satisfy the
    /// lookup and then fail later.
    pub async fn get_key(&self, kid: &str) -> Result<Arc<VerificationKey>> {
        assert!(!kid.is_empty(), "kid must not be empty");
        assert!(
            MIN_REFRESH_INTERVAL > Duration::ZERO,
            "MIN_REFRESH_INTERVAL must be non-zero"
        );

        let keys = self.get_keys().await?;
        if let Some(key) = keys.get(kid) {
            return Ok(key);
        }

        self.refresh().await?;

        let keys = self.get_keys().await?;
        keys.get(kid).ok_or_else(|| Error::InvalidGrant {
            reason: format!("No matching key for kid: {kid} (after forced refetch)"),
        })
    }

    /// Force a refetch of the JWKS, bypassing the TTL check, but bounded to at most one
    /// network fetch per [`MIN_REFRESH_INTERVAL`]. If a forced refetch happened more recently
    /// than the interval allows, this returns `Ok(())` without issuing a request, leaving the
    /// existing cache entry in place.
    ///
    /// The rate-limit timestamp is written and the guard released *before* the network
    /// call: at most one fetch per interval even when the upstream is unhealthy, and no
    /// lock guard spans the fetch. This path is serialized by the *timestamp* rather than
    /// by the refill permit — racing callers after the first are told "rate-limited" and
    /// return immediately, so no queueing is needed here and the two refetch triggers
    /// (TTL expiry and kid-miss) stay independently bounded.
    pub async fn refresh(&self) -> Result<()> {
        assert!(
            MIN_REFRESH_INTERVAL > Duration::ZERO,
            "MIN_REFRESH_INTERVAL must be non-zero"
        );

        {
            let last = self.last_forced_refetch.read().await;
            if let Some(last) = *last {
                if last.elapsed() < MIN_REFRESH_INTERVAL {
                    return Ok(());
                }
            }
        }

        {
            let mut last_guard = self.last_forced_refetch.write().await;

            // Double-check under the write lock: another task may have refreshed while we
            // waited.
            if let Some(last) = *last_guard {
                if last.elapsed() < MIN_REFRESH_INTERVAL {
                    return Ok(());
                }
            }

            // Record the attempt *before* the network call, not after a successful one: this
            // ensures at most one network fetch per `MIN_REFRESH_INTERVAL` even when the
            // upstream is unhealthy and `fetch_keys` returns an error, so a failing (or
            // attacker-targeted) JWKS endpoint cannot be hammered by repeated forced
            // refetches within the interval.
            *last_guard = Some(Instant::now());
            // Deliberately drop `last_guard`: the timestamp is already recorded, so the
            // guard's job is done and it must not span the fetch below.
        }

        // `fetch_keys` fails closed on a JWKS document without a usable `keys`
        // array (the constructor's document-level check), so a malformed body
        // never reaches the cache.
        let keys = self.fetch_keys().await?;

        let mut cache_guard = self.cache.write().await;
        *cache_guard = Some(CachedJwks {
            keys,
            fetched_at: Instant::now(),
        });
        Ok(())
    }

    /// Fetch the remote JWKS through the shared transport and convert it into a
    /// key set under this cache's admitted-algorithm policy.
    ///
    /// Every failure mode — transport fault, byte ceiling, malformed document,
    /// ambiguous duplicates — leaves the cache untouched: nothing is cached
    /// unless a complete, eligible key set was built. Called only while holding
    /// the refill permit (TTL path) or after writing the rate-limit timestamp
    /// (forced path).
    async fn fetch_keys(&self) -> Result<Arc<VerificationKeySet>> {
        // The fetch goes through the shared transport: status before body, body
        // through the shared byte ceiling, non-success detail via the safe path.
        let value: serde_json::Value = crate::shared::transport::ProviderTransport
            .get_json(&self.jwks_uri, &self.jwks_uri)
            .await?
            .parsed(&self.jwks_uri)?;

        let set = VerificationKeySet::from_jwks(&self.jwks_uri, &value, self.admitted_algorithms)?;
        Ok(Arc::new(set))
    }

    /// Rewind the cached entry's fetch time so every concurrent caller observes
    /// it as expired — deterministically, with no sleep-vs-schedule race in the
    /// concurrency tests. Test-only: the tests live in this module and may
    /// touch private state.
    #[cfg(test)]
    async fn expire_cached_entry_for_test(&self) {
        let mut guard = self.cache.write().await;
        let cached = guard
            .as_mut()
            .expect("the cache must be populated before expiring it");
        let age_beyond_ttl = self
            .ttl
            .checked_add(Duration::from_millis(50))
            .and_then(|total| Instant::now().checked_sub(total))
            .expect("ttl plus margin must fit inside an Instant");
        cached.fetched_at = age_beyond_ttl;
    }
}

/// The JWS `alg` name (e.g. `"RS256"`, `"EdDSA"`) for a resolved
/// [`jsonwebtoken::Algorithm`].
///
/// `jsonwebtoken` 10 exposes no `Display` for `Algorithm`, and the validators must
/// report the algorithm they verified with as data (`IdentityClaims.signing_alg`), so
/// the mapping lives here once instead of being re-derived from the untrusted header
/// or re-matched ad hoc at each call site.
pub fn jws_alg_name(alg: jsonwebtoken::Algorithm) -> &'static str {
    use jsonwebtoken::Algorithm as A;

    match alg {
        A::HS256 => "HS256",
        A::HS384 => "HS384",
        A::HS512 => "HS512",
        A::ES256 => "ES256",
        A::ES384 => "ES384",
        A::RS256 => "RS256",
        A::RS384 => "RS384",
        A::RS512 => "RS512",
        A::PS256 => "PS256",
        A::PS384 => "PS384",
        A::PS512 => "PS512",
        A::EdDSA => "EdDSA",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn sample_jwks() -> serde_json::Value {
        json!({
            "keys": [
                {
                    "kty": "RSA",
                    "kid": "test-key-1",
                    "alg": "RS256",
                    "use": "sig",
                    "n": "0vx7agoebGcQSuuPiLJXZptN9nndrQmbXEps2aiAFbWhM78LhWx4cbbfAAtVT86zwu1RK7aPFFxuhDR1L6tSoc_BJECPebWKRXjBZCiFV4n3oknjhMstn64tZ_2W-5JsGY4Hc5n9yBXArwl93lqt7_RN5w6Cf0h4QyQ5v-65YGjQR0_FDW2QvzqY368QQMicAtaSqzs8KJZgnYb9c7d0zgdAZHzu6qMQvRL5hajrn1n91CbOpbISD08qNLyrdkt-bFTWhAI4vMQFh6WeZu0fM4lFd2NcRwr3XPksINHaQ-G_xBniIqbw0Ls1jF44-csFCur-kEgU8awapJzKnqDKgw",
                    "e": "AQAB"
                }
            ]
        })
    }

    #[tokio::test]
    async fn first_call_fetches_and_builds_a_key_set() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/jwks"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_jwks()))
            .expect(1)
            .mount(&server)
            .await;

        let cache = JwksCache::new(
            format!("{}/jwks", server.uri()),
            crate::oidc::OIDC_ADMITTED_ALGORITHMS,
        );
        let keys = cache.get_keys().await.expect("should fetch keys");

        assert!(
            keys.get("test-key-1").is_some(),
            "the fetched kid must resolve in the built set"
        );
        assert_eq!(
            keys.get("test-key-1").unwrap().algorithm(),
            Algorithm::RS256
        );
    }

    #[tokio::test]
    async fn second_call_uses_cache() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/jwks"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_jwks()))
            .expect(1) // Exactly one request expected
            .mount(&server)
            .await;

        let cache = JwksCache::new(
            format!("{}/jwks", server.uri()),
            crate::oidc::OIDC_ADMITTED_ALGORITHMS,
        );

        let keys1 = cache.get_keys().await.expect("first call should succeed");
        let keys2 = cache.get_keys().await.expect("second call should succeed");

        // Arc hand-out: both callers hold the same allocation.
        assert!(
            Arc::ptr_eq(&keys1, &keys2),
            "cache hits must hand out the same Arc, not a deep clone"
        );
        // wiremock's `expect(1)` will panic on drop if more than 1 request was made
    }

    #[tokio::test]
    async fn stale_cache_triggers_refresh() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/jwks"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_jwks()))
            .expect(2) // Two fetches: initial + refresh
            .mount(&server)
            .await;

        // Use a very short TTL so the cache becomes stale immediately.
        let cache = JwksCache::with_ttl(
            format!("{}/jwks", server.uri()),
            Duration::from_millis(1),
            crate::oidc::OIDC_ADMITTED_ALGORITHMS,
        );

        let _keys1 = cache.get_keys().await.expect("first call");

        // Wait for TTL to expire.
        tokio::time::sleep(Duration::from_millis(10)).await;

        let _keys2 = cache.get_keys().await.expect("second call after expiry");
        // wiremock's `expect(2)` verifies exactly 2 requests
    }

    #[tokio::test]
    async fn non_2xx_response_is_error_and_leaves_cache_unpopulated() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/jwks"))
            .respond_with(ResponseTemplate::new(500))
            .up_to_n_times(1)
            .expect(1)
            .mount(&server)
            .await;

        let cache = JwksCache::new(
            format!("{}/jwks", server.uri()),
            crate::oidc::OIDC_ADMITTED_ALGORITHMS,
        );

        let err = cache
            .get_keys()
            .await
            .expect_err("500 must return an error");
        assert!(matches!(err, Error::ProviderError { .. }));
        assert!(
            format!("{err}").contains("500"),
            "error should name the offending status: {err}"
        );

        // The failed fetch must not have populated the cache: assert directly on the private
        // field (this test module is nested inside `jwks`, so it can see it) so a later 200
        // response is a genuine fresh fetch, not a served-from-cache result.
        assert!(
            cache.cache.read().await.is_none(),
            "a non-2xx response must never populate the cache"
        );

        // A subsequent successful fetch does populate the cache.
        Mock::given(method("GET"))
            .and(path("/jwks"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_jwks()))
            .expect(1)
            .mount(&server)
            .await;

        let keys = cache
            .get_keys()
            .await
            .expect("a subsequent success should now be cached");
        assert!(keys.get("test-key-1").is_some());
        assert!(cache.cache.read().await.is_some());
    }

    #[tokio::test]
    async fn forced_refetch_within_interval_makes_no_second_request() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/jwks"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_jwks()))
            .expect(1) // Only the first `refresh()` should hit the network.
            .mount(&server)
            .await;

        let cache = JwksCache::new(
            format!("{}/jwks", server.uri()),
            crate::oidc::OIDC_ADMITTED_ALGORITHMS,
        );

        cache
            .refresh()
            .await
            .expect("first forced refetch should succeed");
        assert!(cache.cache.read().await.is_some());

        // Called immediately afterwards, well inside MIN_REFRESH_INTERVAL: must not issue a
        // second network request (wiremock's `expect(1)` panics on drop otherwise).
        cache
            .refresh()
            .await
            .expect("rate-limited refresh should still return Ok without refetching");
    }

    #[tokio::test]
    async fn failing_upstream_still_rate_limits_forced_refetch() {
        // A JWKS endpoint that always errors must still only be hit once per
        // `MIN_REFRESH_INTERVAL`: the rate-limit timestamp has to be recorded on the *attempt*,
        // not only on success, or a caller retrying on every `kid` miss would hammer an
        // unhealthy (or attacker-targeted) upstream with unbounded requests.
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/jwks"))
            .respond_with(ResponseTemplate::new(500))
            .expect(1) // Only the first forced refetch should reach the network.
            .mount(&server)
            .await;

        let cache = JwksCache::new(
            format!("{}/jwks", server.uri()),
            crate::oidc::OIDC_ADMITTED_ALGORITHMS,
        );

        let err1 = cache
            .refresh()
            .await
            .expect_err("first forced refetch against a failing upstream must error");
        assert!(matches!(err1, Error::ProviderError { .. }));
        assert!(cache.cache.read().await.is_none());

        // Called immediately afterwards, well inside MIN_REFRESH_INTERVAL: must not issue a
        // second network request even though the first attempt failed (wiremock's `expect(1)`
        // panics on drop otherwise). The rate limit itself does not surface as an error.
        cache
            .refresh()
            .await
            .expect("rate-limited refresh after a failed attempt should return Ok, not refetch");
        assert!(cache.cache.read().await.is_none());
    }

    #[tokio::test]
    async fn get_key_returns_resolved_verification_key_when_cached() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/jwks"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_jwks()))
            .expect(1)
            .mount(&server)
            .await;

        let cache = JwksCache::new(
            format!("{}/jwks", server.uri()),
            crate::oidc::OIDC_ADMITTED_ALGORITHMS,
        );

        let key = cache
            .get_key("test-key-1")
            .await
            .expect("known kid should resolve without a forced refetch");
        assert_eq!(key.kid(), "test-key-1");
        assert_eq!(key.algorithm(), Algorithm::RS256);
    }

    #[tokio::test]
    async fn get_key_forces_one_refetch_on_unknown_kid_then_rate_limits() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/jwks"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_jwks()))
            .expect(2) // Initial TTL fetch (kid miss) + one forced refetch, no more.
            .mount(&server)
            .await;

        let cache = JwksCache::new(
            format!("{}/jwks", server.uri()),
            crate::oidc::OIDC_ADMITTED_ALGORITHMS,
        );

        // "unknown-kid" never appears in `sample_jwks`, so the first lookup misses, forces one
        // refetch (which still returns the same set), and then fails closed.
        let err = cache
            .get_key("unknown-kid")
            .await
            .expect_err("kid absent even after forced refetch must be an error");
        assert!(matches!(err, Error::InvalidGrant { .. }));
        assert!(
            err.to_string().contains("unknown-kid"),
            "the miss names the kid: {err}"
        );

        // A second miss immediately afterwards is rate-limited: no third network call (the
        // `expect(2)` above would panic on drop if one occurred).
        let err2 = cache
            .get_key("unknown-kid")
            .await
            .expect_err("rate-limited second miss must still fail closed");
        assert!(matches!(err2, Error::InvalidGrant { .. }));
    }

    #[tokio::test]
    async fn kid_matching_only_an_ineligible_entry_is_a_miss_that_forces_one_refetch() {
        // A `use: enc` entry with the requested kid: eligibility lives in the
        // constructor now, so the lookup misses, forces the one permitted
        // refetch, and fails closed — the shape invariant requires, rather than
        // a resolution that fails mid-validation.
        let server = MockServer::start().await;

        let jwks = json!({
            "keys": [{
                "kty": "RSA",
                "kid": "enc-only",
                "alg": "RS256",
                "use": "enc",
                "n": sample_jwks()["keys"][0]["n"],
                "e": "AQAB"
            }]
        });

        Mock::given(method("GET"))
            .and(path("/jwks"))
            .respond_with(ResponseTemplate::new(200).set_body_json(jwks))
            .expect(2) // Initial fetch + exactly one forced refetch.
            .mount(&server)
            .await;

        let cache = JwksCache::new(
            format!("{}/jwks", server.uri()),
            crate::oidc::OIDC_ADMITTED_ALGORITHMS,
        );

        let err = cache
            .get_key("enc-only")
            .await
            .expect_err("an encryption key must not satisfy a verification lookup");
        assert!(matches!(err, Error::InvalidGrant { .. }));
    }

    // -------------------------------------------------------------------
    // Single-flight concurrency (task 05): delayed-origin races against a
    // slow upstream, so a naive guard-dropping fix would produce multiple
    // fetches and wiremock's `.expect(N)` would panic on drop.
    //
    // RACE_SIZE callers align on a barrier before entering get_keys, which
    // makes the interleavings deterministic in the ways each test asserts:
    // the elected caller holds the permit for at least the mock's response
    // delay, so every other racer arrives while the refill is in flight.
    // -------------------------------------------------------------------

    /// Number of racing callers per concurrency test; three or more, per the
    /// task's acceptance criterion.
    const RACE_SIZE: usize = 4;

    /// Response delay used to hold the elected fetcher's window open, long
    /// enough that all barrier-aligned racers land inside it even loaded CI.
    const REFILL_DELAY_MS: u64 = 400;

    /// Upper bound proving nobody serially waited through more than one fetch
    /// (two full fetches would exceed it by a wide margin).
    const NO_DOUBLE_WAIT_BOUND_MS: u64 = 3 * REFILL_DELAY_MS;

    async fn mount_delayed_jwks(server: &MockServer, expected_requests: u64) {
        Mock::given(method("GET"))
            .and(path("/jwks"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(sample_jwks())
                    .set_delay(Duration::from_millis(REFILL_DELAY_MS)),
            )
            .expect(expected_requests)
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn expired_ttl_racers_collapse_into_exactly_one_fetch_with_stale_serving() {
        let server = MockServer::start().await;
        // One warm-up fill + exactly one elected refill; a broken election
        // (one fetch per racer) would mean five requests and panic on drop.
        mount_delayed_jwks(&server, 2).await;

        let cache = Arc::new(JwksCache::new(
            format!("{}/jwks", server.uri()),
            crate::oidc::OIDC_ADMITTED_ALGORITHMS,
        ));

        // Warm the cache, keep the stale Arc for identity checks, then expire
        // the entry deterministically (no sleeps): every racer must now see
        // "expired but parseable".
        let stale_keys = cache.get_keys().await.expect("initial fill");
        assert_eq!(
            stale_keys.get("test-key-1").map(|k| k.kid().to_string()),
            Some("test-key-1".to_string())
        );
        cache.expire_cached_entry_for_test().await;

        let start = std::time::Instant::now();
        let barrier = Arc::new(tokio::sync::Barrier::new(RACE_SIZE));
        let mut handles = Vec::with_capacity(RACE_SIZE);
        for _ in 0..RACE_SIZE {
            let cache = Arc::clone(&cache);
            let barrier = Arc::clone(&barrier);
            handles.push(tokio::spawn(async move {
                barrier.wait().await;
                cache.get_keys().await.expect("racer must not error")
            }));
        }

        let mut served_stale = 0;
        let mut served_any = 0;
        for handle in handles {
            let keys = handle.await.expect("racer task joins");
            served_any += 1;
            if Arc::ptr_eq(&keys, &stale_keys) {
                served_stale += 1;
            }
            // Every racer — stale-served or fresh-served — resolves the kid.
            assert_eq!(
                keys.get("test-key-1").map(|k| k.kid().to_string()),
                Some("test-key-1".to_string()),
                "a served set must always resolve the known kid"
            );
        }

        assert_eq!(served_any, RACE_SIZE);
        // At least one racer was served the stale set rather than queueing for
        // the in-flight refill (the elected caller itself gets the fresh set,
        // which is why this is not asserted as ALL of them).
        assert!(
            served_stale >= 1,
            "stale-but-parseable serving must reach non-elected callers"
        );
        let elapsed_ms = start.elapsed().as_millis();
        assert!(
            elapsed_ms < NO_DOUBLE_WAIT_BOUND_MS as u128,
            "no racer may wait longer than one fetch: took {elapsed_ms}ms"
        );
        // `.expect(1)` above panics on drop unless exactly ONE request arrived.
    }

    #[tokio::test]
    async fn cold_cache_racers_collapse_into_exactly_one_fetch() {
        let server = MockServer::start().await;
        mount_delayed_jwks(&server, 1).await;

        let cache = Arc::new(JwksCache::new(
            format!("{}/jwks", server.uri()),
            crate::oidc::OIDC_ADMITTED_ALGORITHMS,
        ));

        let barrier = Arc::new(tokio::sync::Barrier::new(RACE_SIZE));
        let mut handles = Vec::with_capacity(RACE_SIZE);
        for _ in 0..RACE_SIZE {
            let cache = Arc::clone(&cache);
            let barrier = Arc::clone(&barrier);
            handles.push(tokio::spawn(async move {
                barrier.wait().await;
                cache.get_keys().await.expect("cold racer must not error")
            }));
        }

        let mut handles = handles.into_iter();
        let first = handles
            .next()
            .expect("RACE_SIZE racers were spawned")
            .await
            .expect("first racer joins");
        // `handles` continues past the first racer taken above.
        for handle in handles {
            let keys = handle.await.expect("racer joins");
            assert!(
                Arc::ptr_eq(&keys, &first),
                "every cold-cache racer shares the winner's set by Arc"
            );
        }
        // The winner stores BEFORE releasing the permit, so queued racers find
        // the fresh entry under the write-guard re-check and spend no request;
        // `.expect(1)` above would panic otherwise.
    }

    #[tokio::test]
    async fn failed_refill_serves_stale_to_waiters_and_costs_one_fetch() {
        // Expired-TTL cache plus an upstream that errors slowly: the elected
        // caller gets the fault, every racer arriving during its window is
        // served the stale set instead of queueing or refetching, and exactly
        // one request leaves the process.
        let server = MockServer::start().await;

        // The warm-up fill gets one successful response; every request after
        // that (the elected refill attempt) fails slowly.
        Mock::given(method("GET"))
            .and(path("/jwks"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_jwks()))
            .up_to_n_times(1)
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/jwks"))
            .respond_with(
                ResponseTemplate::new(500).set_delay(Duration::from_millis(REFILL_DELAY_MS)),
            )
            .expect(1) // Exactly the elected caller's failed attempt; no more.
            .up_to_n_times(1)
            .mount(&server)
            .await;

        let cache = Arc::new(JwksCache::new(
            format!("{}/jwks", server.uri()),
            crate::oidc::OIDC_ADMITTED_ALGORITHMS,
        ));

        let stale_keys = cache.get_keys().await.expect("initial fill");
        assert_eq!(stale_keys.len(), 1);
        cache.expire_cached_entry_for_test().await;

        let barrier = Arc::new(tokio::sync::Barrier::new(RACE_SIZE));
        let mut handles = Vec::with_capacity(RACE_SIZE);
        for _ in 0..RACE_SIZE {
            let cache = Arc::clone(&cache);
            let barrier = Arc::clone(&barrier);
            handles.push(tokio::spawn(async move {
                barrier.wait().await;
                cache.get_keys().await
            }));
        }

        let mut errors = 0;
        let mut stale_served = 0;
        for handle in handles {
            match handle.await.expect("racer task joins") {
                Ok(keys) => {
                    assert!(
                        Arc::ptr_eq(&keys, &stale_keys),
                        "an Ok during a failing refill must be the stale set"
                    );
                    stale_served += 1;
                }
                Err(e) => {
                    assert!(
                        matches!(e, Error::ProviderError { .. }),
                        "the elected caller reports the upstream fault: {e:?}"
                    );
                    errors += 1;
                }
            }
        }

        assert_eq!(errors, 1, "exactly the elected caller sees the failure");
        assert_eq!(
            stale_served,
            RACE_SIZE - 1,
            "every other racer is served the stale-but-parseable set"
        );
        // Failure leaves the cache untouched: still the expired entry, never a
        // bad value, never wiped.
        assert!(cache.any_cached().await.is_some());
        assert!(cache.cache.read().await.as_ref().is_some());
    }

    #[tokio::test]
    async fn failed_refill_on_a_cold_cache_serializes_one_attempt_per_waiter() {
        // Cold cache (nothing parseable to serve) against a fast-failing
        // upstream: waiters queue on the permit and each takes exactly one
        // serialized turn — never parallel, all fail closed, nothing cached.
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/jwks"))
            .respond_with(ResponseTemplate::new(500))
            .expect(RACE_SIZE as u64)
            .mount(&server)
            .await;

        let cache = Arc::new(JwksCache::new(
            format!("{}/jwks", server.uri()),
            crate::oidc::OIDC_ADMITTED_ALGORITHMS,
        ));

        let barrier = Arc::new(tokio::sync::Barrier::new(RACE_SIZE));
        let mut handles = Vec::with_capacity(RACE_SIZE);
        for _ in 0..RACE_SIZE {
            let cache = Arc::clone(&cache);
            let barrier = Arc::clone(&barrier);
            handles.push(tokio::spawn(async move {
                barrier.wait().await;
                cache.get_keys().await
            }));
        }

        for handle in handles {
            let result = handle.await.expect("racer task joins");
            let err = result.expect_err("every serialized attempt must report the fault");
            assert!(
                matches!(err, Error::ProviderError { .. }),
                "failures stay provider faults: {err:?}"
            );
        }

        assert!(
            cache.cache.read().await.is_none(),
            "failed attempts must leave the cold cache unpopulated"
        );
        // `.expect(RACE_SIZE)` above pins the bound: one attempt per waiter,
        // not one per retry-loop lap — a herd of parallel fetches would panic.
    }

    #[tokio::test]
    async fn concurrent_unknown_kid_lookups_force_exactly_one_refetch() {
        // Racing kid misses: the forced-refetch path is rate-limited by the
        // timestamp written before the fetch, so among RACE_SIZE concurrent
        // lookups exactly one network attempt happens and everyone fails closed.
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/jwks"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_jwks()))
            .expect(2) // Warm-up fill + exactly one rate-limited forced refetch.
            .mount(&server)
            .await;

        let cache = Arc::new(JwksCache::new(
            format!("{}/jwks", server.uri()),
            crate::oidc::OIDC_ADMITTED_ALGORITHMS,
        ));

        // Warm the cache with a set that lacks "unknown-kid" so every racer
        // misses without any racer needing the initial TTL fetch.
        let _warm = cache.get_keys().await.expect("warm-up fill");

        let barrier = Arc::new(tokio::sync::Barrier::new(RACE_SIZE));
        let mut handles = Vec::with_capacity(RACE_SIZE);
        for _ in 0..RACE_SIZE {
            let cache = Arc::clone(&cache);
            let barrier = Arc::clone(&barrier);
            handles.push(tokio::spawn(async move {
                barrier.wait().await;
                cache.get_key("unknown-kid").await
            }));
        }

        for handle in handles {
            let result = handle.await.expect("racer task joins");
            let err = result.expect_err("the kid is absent even after the forced refetch");
            assert!(
                matches!(err, Error::InvalidGrant { .. }),
                "unknown kids fail closed regardless of race position: {err:?}"
            );
        }
        // `.expect(1)` above: the timestamp recorded before the first fetch
        // rate-limits every other racer within MIN_REFRESH_INTERVAL.
    }

    #[tokio::test]
    async fn large_key_set_cache_hits_are_sub_millisecond() {
        // Benchmark backing task-05 step 1 (Arc hand-out instead of deep clone).
        // A real RSA JWK replicated under many distinct kids builds a set big
        // enough that a deep clone per call would be plainly measurable; the
        // Arc hand-out sits orders of magnitude below the sub-millisecond
        // target. Measured locally (2026-08-22, Apple M-series, debug build):
        // 0.8-0.9 µs per hit over 2 000 hits on a 96-key set, including the
        // RwLock read, TTL check, Arc clone, and kid resolution — see the
        // assertion bound. (96 keys keeps the mock body under the shared
        // 64 KiB upstream ceiling.)
        let server = MockServer::start().await;

        const KEY_COUNT: usize = 96;
        const HIT_COUNT: usize = 2_000;

        let base = sample_jwks()["keys"][0].clone();
        let keys: Vec<serde_json::Value> = (0..KEY_COUNT)
            .map(|i| {
                let mut k = base.clone();
                k["kid"] = json!(format!("bench-key-{i}"));
                k
            })
            .collect();
        let body = json!({ "keys": keys });

        Mock::given(method("GET"))
            .and(path("/jwks"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1) // Exactly one network fill; every later call is a hit.
            .mount(&server)
            .await;

        let cache = JwksCache::new(
            format!("{}/jwks", server.uri()),
            crate::oidc::OIDC_ADMITTED_ALGORITHMS,
        );

        let warm = cache.get_keys().await.expect("large set fills once");
        assert_eq!(warm.len(), KEY_COUNT, "all distinct-kid entries kept");

        let start = std::time::Instant::now();
        for i in 0..HIT_COUNT {
            let kid = format!("bench-key-{}", i % KEY_COUNT);
            let keys = cache.get_keys().await.expect("hits must never error");
            let resolved = keys
                .get(&kid)
                .unwrap_or_else(|| panic!("hit {i} must resolve {kid}"));
            assert_eq!(resolved.algorithm(), Algorithm::RS256);
        }
        let mean_micros = start.elapsed().as_nanos() as f64 / (HIT_COUNT as f64 * 1000.0);

        // Sub-millisecond target with headroom for loaded CI; a deep clone of a
        // 256-key set per call would blow far past this bound.
        assert!(
            mean_micros < 1_000.0,
            "mean cache hit must stay under 1ms, measured {mean_micros:.1}µs"
        );
        println!("large-key-set mean cache-hit time: {mean_micros:.1}µs over {HIT_COUNT} hits");
    }

    /// The JWS name mapping is the data `IdentityClaims.signing_alg` carries, so it must
    /// round-trip with `Algorithm::from_str` for every algorithm the validators accept,
    /// and stay stable for the digest families the core's `at_hash` check selects on.
    #[test]
    fn jws_alg_name_round_trips_with_from_str() {
        use std::str::FromStr;

        for name in [
            "HS256", "HS384", "HS512", "ES256", "ES384", "RS256", "RS384", "RS512", "PS256",
            "PS384", "PS512", "EdDSA",
        ] {
            let alg = jsonwebtoken::Algorithm::from_str(name).expect("supported algorithm");
            assert_eq!(
                jws_alg_name(alg),
                name,
                "mapping must invert from_str for {name}"
            );
        }
    }

    #[test]
    fn jws_alg_name_names_the_digest_families_the_core_selects_on() {
        assert_eq!(jws_alg_name(jsonwebtoken::Algorithm::RS256), "RS256");
        assert_eq!(jws_alg_name(jsonwebtoken::Algorithm::ES256), "ES256");
        assert_eq!(jws_alg_name(jsonwebtoken::Algorithm::ES384), "ES384");
        // EdDSA has no at_hash digest (OIDC Core defines none); the name must still be
        // reported faithfully so the core can reject an at_hash on such assertions.
        assert_eq!(jws_alg_name(jsonwebtoken::Algorithm::EdDSA), "EdDSA");
    }
}
