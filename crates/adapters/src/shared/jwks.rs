use std::sync::Arc;
use std::time::{Duration, Instant};

use oidc_exchange_core::error::{Error, Result};
use tokio::sync::RwLock;

/// Default TTL for JWKS cache entries: 1 hour.
const DEFAULT_TTL: Duration = Duration::from_secs(3600);

/// Minimum interval between forced refetches triggered by a `kid` cache miss: 30 seconds.
///
/// Bounds how often a caller can force a network fetch outside the normal TTL-based refresh,
/// so an attacker spraying unknown `kid`s cannot turn the service into a JWKS-endpoint
/// hammer.
const MIN_REFRESH_INTERVAL: Duration = Duration::from_secs(30);

/// Fetches and caches a JWKS key set from a remote URL with TTL-based refresh.
pub struct JwksCache {
    jwks_uri: String,
    cache: Arc<RwLock<Option<CachedJwks>>>,
    ttl: Duration,
    /// Instant of the last forced refetch, guarding [`MIN_REFRESH_INTERVAL`]. `None` until the
    /// first forced refetch happens.
    last_forced_refetch: Arc<RwLock<Option<Instant>>>,
}

struct CachedJwks {
    keys: serde_json::Value,
    fetched_at: Instant,
}

impl JwksCache {
    /// Create a new `JwksCache` with the default TTL of 1 hour.
    pub fn new(jwks_uri: String) -> Self {
        Self {
            jwks_uri,
            cache: Arc::new(RwLock::new(None)),
            ttl: DEFAULT_TTL,
            last_forced_refetch: Arc::new(RwLock::new(None)),
        }
    }

    /// Create a new `JwksCache` with a custom TTL.
    pub fn with_ttl(jwks_uri: String, ttl: Duration) -> Self {
        Self {
            jwks_uri,
            cache: Arc::new(RwLock::new(None)),
            ttl,
            last_forced_refetch: Arc::new(RwLock::new(None)),
        }
    }

    /// Return the cached JWKS if still fresh, otherwise fetch from the remote URL.
    ///
    /// No lock that protects the cached value is held across the fetch: the
    /// slow path checks freshness under the write guard, releases it, fetches
    /// outside any lock, and re-acquires only to store. The interim cost of
    /// that ordering is a possible thundering herd of refetches (bounded per
    /// fetch by the shared byte ceiling), which the single-flight redesign
    /// replaces with an elected fetcher.
    pub async fn get_keys(&self) -> Result<serde_json::Value> {
        // Fast path: read lock to check if cache is valid.
        {
            let guard = self.cache.read().await;
            if let Some(ref cached) = *guard {
                if cached.fetched_at.elapsed() < self.ttl {
                    return Ok(cached.keys.clone());
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
                    return Ok(cached.keys.clone());
                }
            }
            // Deliberately drop `guard` here: fetching under it is the
            // lock-across-network defect this ordering removes.
        }

        let keys = self.fetch_keys().await?;

        let mut guard = self.cache.write().await;
        *guard = Some(CachedJwks {
            keys: keys.clone(),
            fetched_at: Instant::now(),
        });
        Ok(keys)
    }

    /// Return the JWK matching `kid`, forcing at most one network refetch per
    /// [`MIN_REFRESH_INTERVAL`] when `kid` is not present in the cached (or freshly fetched)
    /// key set.
    ///
    /// This is distinct from the TTL-based refresh in [`get_keys`](Self::get_keys): it exists
    /// so a legitimate key rotation is picked up without waiting out the (much longer)
    /// `ttl`, while still bounding how often an attacker spraying unknown `kid`s can force a
    /// fetch.
    pub async fn get_key(&self, kid: &str) -> Result<serde_json::Value> {
        assert!(!kid.is_empty(), "kid must not be empty");
        assert!(
            MIN_REFRESH_INTERVAL > Duration::ZERO,
            "MIN_REFRESH_INTERVAL must be non-zero"
        );

        let keys = self.get_keys().await?;
        if let Some(key) = find_key(&keys, kid) {
            return Ok(key);
        }

        self.refresh().await?;

        let keys = {
            let guard = self.cache.read().await;
            guard
                .as_ref()
                .map(|cached| cached.keys.clone())
                .unwrap_or(keys)
        };

        find_key(&keys, kid).ok_or_else(|| Error::ProviderError {
            provider: self.jwks_uri.clone(),
            detail: format!("no JWK found for kid {kid:?} after forced refetch"),
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

        let keys = self.fetch_keys().await?;
        if keys.get("keys").is_none() {
            return Err(Error::ProviderError {
                provider: self.jwks_uri.clone(),
                detail: "JWKS response body is missing the required 'keys' field".to_string(),
            });
        }

        let mut cache_guard = self.cache.write().await;
        *cache_guard = Some(CachedJwks {
            keys,
            fetched_at: Instant::now(),
        });
        Ok(())
    }

    async fn fetch_keys(&self) -> Result<serde_json::Value> {
        // The fetch goes through the shared transport: status before body, body
        // through the shared byte ceiling, non-success detail via the safe path.
        let keys: serde_json::Value = crate::shared::transport::ProviderTransport
            .get_json(&self.jwks_uri, &self.jwks_uri)
            .await?
            .parsed(&self.jwks_uri)?;

        if !keys.is_object() {
            return Err(Error::ProviderError {
                provider: self.jwks_uri.clone(),
                detail: "JWKS response body is not a JSON object".to_string(),
            });
        }
        Ok(keys)
    }
}

/// Find the JWK with the given `kid` inside a JWKS `keys` array, if present.
fn find_key(jwks: &serde_json::Value, kid: &str) -> Option<serde_json::Value> {
    jwks.get("keys")?
        .as_array()?
        .iter()
        .find(|key| key.get("kid").and_then(|v| v.as_str()) == Some(kid))
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn sample_jwks() -> serde_json::Value {
        serde_json::json!({
            "keys": [
                {
                    "kty": "RSA",
                    "kid": "test-key-1",
                    "n": "0vx7agoebGcQSuuPiLJXZptN9nndrQmbXEps2aiAFbWhM78LhWx4cbbfAAtVT86zwu1RK7aPFFxuhDR1L6tSoc_BJECPebWKRXjBZCiFV4n3oknjhMstn64tZ_2W-5JsGY4Hc5n9yBXArwl93lqt7_RN5w6Cf0h4QyQ5v-65YGjQR0_FDW2QvzqY368QQMicAtaSqzs8KJZgnYb9c7d0zgdAZHzu6qMQvRL5hajrn1n91CbOpbISD08qNLyrdkt-bFTWhAI4vMQFh6WeZu0fM4lFd2NcRwr3XPksINHaQ-G_xBniIqbw0Ls1jF44-csFCur-kEgU8awapJzKnqDKgw",
                    "e": "AQAB"
                }
            ]
        })
    }

    #[tokio::test]
    async fn first_call_fetches_from_url() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/jwks"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_jwks()))
            .expect(1)
            .mount(&server)
            .await;

        let cache = JwksCache::new(format!("{}/jwks", server.uri()));
        let keys = cache.get_keys().await.expect("should fetch keys");

        assert!(keys["keys"].is_array());
        assert_eq!(keys["keys"][0]["kid"], "test-key-1");
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

        let cache = JwksCache::new(format!("{}/jwks", server.uri()));

        let keys1 = cache.get_keys().await.expect("first call should succeed");
        let keys2 = cache.get_keys().await.expect("second call should succeed");

        assert_eq!(keys1, keys2);
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
        let cache = JwksCache::with_ttl(format!("{}/jwks", server.uri()), Duration::from_millis(1));

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

        let cache = JwksCache::new(format!("{}/jwks", server.uri()));

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
        assert_eq!(keys["keys"][0]["kid"], "test-key-1");
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

        let cache = JwksCache::new(format!("{}/jwks", server.uri()));

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

        let cache = JwksCache::new(format!("{}/jwks", server.uri()));

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
    async fn get_key_returns_matching_key_without_refetch_when_cached() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/jwks"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_jwks()))
            .expect(1)
            .mount(&server)
            .await;

        let cache = JwksCache::new(format!("{}/jwks", server.uri()));

        let key = cache
            .get_key("test-key-1")
            .await
            .expect("known kid should resolve without a forced refetch");
        assert_eq!(key["kid"], "test-key-1");
        assert_eq!(key["kty"], "RSA");
    }

    /// Serve one HTTP/1.1 chunked response from a raw TCP listener. wiremock
    /// cannot force chunked encoding, and chunked is exactly the case where an
    /// honest `Content-Length` short-circuit does not apply.
    async fn spawn_chunked_jwks_server(total_bytes: usize, chunk_size: usize) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("ephemeral bind works");
        let addr = listener.local_addr().expect("local_addr resolves");

        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("one connection");
            let mut scratch = [0u8; 4096];
            loop {
                let read = socket.read(&mut scratch).await.expect("readable");
                if read == 0 || scratch[..read].windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
                .await
                .expect("head writable");
            let mut remaining = total_bytes;
            while remaining > 0 {
                let n = remaining.min(chunk_size);
                socket
                    .write_all(format!("{n:x}\r\n").as_bytes())
                    .await
                    .expect("chunk header writable");
                socket
                    .write_all(&vec![b'a'; n])
                    .await
                    .expect("chunk writable");
                socket
                    .write_all(b"\r\n")
                    .await
                    .expect("chunk CRLF writable");
                remaining -= n;
            }
            socket
                .write_all(b"0\r\n\r\n")
                .await
                .expect("terminal chunk writable");
        });

        format!("http://{addr}/jwks")
    }

    #[tokio::test]
    async fn oversized_jwks_with_honest_content_length_is_a_cap_error_and_leaves_cache_unpopulated()
    {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/jwks"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                "k".repeat(crate::shared::http::MAX_UPSTREAM_BODY_BYTES as usize + 1),
            ))
            .expect(1)
            .mount(&server)
            .await;

        let cache = JwksCache::new(format!("{}/jwks", server.uri()));

        let err = cache
            .get_keys()
            .await
            .expect_err("an over-ceiling JWKS body must fail");
        assert!(
            format!("{err}").contains("exceeded the"),
            "must be the distinctive cap error, got: {err}"
        );
        // The failed fetch must not populate the cache, exactly as for a non-2xx:
        // a cap failure leaves no half-fetched state to be served later.
        assert!(
            cache.cache.read().await.is_none(),
            "an over-limit response must never populate the cache"
        );
    }

    #[tokio::test]
    async fn oversized_chunked_jwks_hits_the_cap_and_never_populates_the_cache() {
        // No Content-Length at all: only the streaming running-total bound can
        // catch this, which is why the ceiling must apply mid-stream too.
        let url = spawn_chunked_jwks_server(
            crate::shared::http::MAX_UPSTREAM_BODY_BYTES as usize * 2,
            8192,
        )
        .await;

        let cache = JwksCache::new(url);

        let err = cache
            .get_keys()
            .await
            .expect_err("a chunked over-ceiling JWKS must fail");
        assert!(
            matches!(err, Error::ProviderError { .. }),
            "the cap failure surfaces as a provider fault, got: {err:?}"
        );
        assert!(
            format!("{err}").contains("exceeded the"),
            "must name the byte ceiling, got: {err}"
        );
        assert!(
            cache.cache.read().await.is_none(),
            "no cache entry may exist after a capped fetch"
        );
    }

    #[tokio::test]
    async fn jwks_within_ceiling_still_fetches_normally() {
        // Boundary check one below the failure: a normal-sized JWKS keeps working
        // through the transport after the ceiling exists (negative space for the
        // two cap tests above).
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/jwks"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_jwks()))
            .expect(1)
            .mount(&server)
            .await;

        let cache = JwksCache::new(format!("{}/jwks", server.uri()));
        let keys = cache.get_keys().await.expect("small JWKS must still parse");
        assert_eq!(keys["keys"][0]["kid"], "test-key-1");
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

        let cache = JwksCache::new(format!("{}/jwks", server.uri()));

        // "unknown-kid" never appears in `sample_jwks`, so the first lookup misses, forces one
        // refetch (which still returns the same set), and then fails closed.
        let err = cache
            .get_key("unknown-kid")
            .await
            .expect_err("kid absent even after forced refetch must be an error");
        assert!(matches!(err, Error::ProviderError { .. }));

        // A second miss immediately afterwards is rate-limited: no third network call (the
        // `expect(2)` above would panic on drop if one occurred).
        let err2 = cache
            .get_key("unknown-kid")
            .await
            .expect_err("rate-limited second miss must still fail closed");
        assert!(matches!(err2, Error::ProviderError { .. }));
    }
}
