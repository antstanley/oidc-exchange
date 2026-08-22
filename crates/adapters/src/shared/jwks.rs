use std::sync::Arc;
use std::time::{Duration, Instant};

use jsonwebtoken::Algorithm;
use oidc_exchange_core::error::{Error, Result};
use tokio::sync::RwLock;

use crate::shared::keys::{VerificationKey, VerificationKeySet};

/// Default TTL for JWKS cache entries: 1 hour.
const DEFAULT_TTL: Duration = Duration::from_secs(3600);

/// Minimum interval between forced refetches triggered by a `kid` cache miss: 30 seconds.
///
/// Bounds how often a caller can force a network fetch outside the normal TTL-based refresh,
/// so an attacker spraying unknown `kid`s cannot turn the service into a JWKS-endpoint
/// hammer.
const MIN_REFRESH_INTERVAL: Duration = Duration::from_secs(30);

/// Fetches and caches a remote JWKS as a [`VerificationKeySet`] with TTL-based refresh.
///
/// The cached value is held behind an `Arc` and handed out by cheap clone; the
/// per-request cost of a cache hit is a pointer bump, not a deep copy of the key
/// set. The cache builds key sets with the caller's admitted-algorithm policy,
/// so eligibility is decided once per fetch, not once per validation.
pub struct JwksCache {
    jwks_uri: String,
    admitted_algorithms: &'static [Algorithm],
    cache: Arc<RwLock<Option<CachedJwks>>>,
    ttl: Duration,
    /// Instant of the last forced refetch, guarding [`MIN_REFRESH_INTERVAL`]. `None` until the
    /// first forced refetch happens.
    last_forced_refetch: Arc<RwLock<Option<Instant>>>,
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
        }
    }

    /// Return the cached key set if still fresh, otherwise fetch from the remote URL.
    ///
    /// No lock that protects the cached value is held across the fetch: the
    /// slow path checks freshness under the write guard, releases it, fetches
    /// outside any lock, and re-acquires only to store. The interim cost of
    /// that ordering is a possible thundering herd of refetches (bounded per
    /// fetch by the shared byte ceiling), which the single-flight redesign
    /// replaces with an elected fetcher.
    pub async fn get_keys(&self) -> Result<Arc<VerificationKeySet>> {
        // Fast path: read lock to check if cache is valid.
        {
            let guard = self.cache.read().await;
            if let Some(ref cached) = *guard {
                if cached.fetched_at.elapsed() < self.ttl {
                    return Ok(Arc::clone(&cached.keys));
                }
            }
        }

        // Slow path: confirm staleness under the write lock, then release it
        // before any network I/O — the guard must never span the fetch.
        {
            let guard = self.cache.write().await;
            if let Some(ref cached) = *guard {
                if cached.fetched_at.elapsed() < self.ttl {
                    // Another task refreshed while we waited for the lock.
                    return Ok(Arc::clone(&cached.keys));
                }
            }
            // Deliberately drop `guard` here: fetching under it is the
            // lock-across-network defect this ordering removes.
        }

        let keys = self.fetch_keys().await?;

        let mut guard = self.cache.write().await;
        *guard = Some(CachedJwks {
            keys: Arc::clone(&keys),
            fetched_at: Instant::now(),
        });
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
    /// lock guard spans the fetch. Callers that arrive while a permitted refetch is in
    /// flight are told "rate-limited" rather than queued behind it; the single-flight
    /// redesign elects one fetcher for that window instead.
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
    /// unless a complete, eligible key set was built.
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
}
