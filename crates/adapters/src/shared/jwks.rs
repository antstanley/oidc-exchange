use std::sync::Arc;
use std::time::{Duration, Instant};

use oidc_exchange_core::error::{Error, Result};
use tokio::sync::RwLock;

/// Default TTL for JWKS cache entries: 1 hour.
const DEFAULT_TTL: Duration = Duration::from_secs(3600);

/// Fetches and caches a JWKS key set from a remote URL with TTL-based refresh.
pub struct JwksCache {
    jwks_uri: String,
    cache: Arc<RwLock<Option<CachedJwks>>>,
    ttl: Duration,
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
        }
    }

    /// Create a new `JwksCache` with a custom TTL.
    pub fn with_ttl(jwks_uri: String, ttl: Duration) -> Self {
        Self {
            jwks_uri,
            cache: Arc::new(RwLock::new(None)),
            ttl,
        }
    }

    /// Return the cached JWKS if still fresh, otherwise fetch from the remote URL.
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

        // Slow path: acquire write lock and fetch.
        let mut guard = self.cache.write().await;

        // Double-check: another task may have refreshed while we waited for the write lock.
        if let Some(ref cached) = *guard {
            if cached.fetched_at.elapsed() < self.ttl {
                return Ok(cached.keys.clone());
            }
        }

        let keys = self.fetch_keys().await?;
        *guard = Some(CachedJwks {
            keys: keys.clone(),
            fetched_at: Instant::now(),
        });
        Ok(keys)
    }

    async fn fetch_keys(&self) -> Result<serde_json::Value> {
        let response =
            reqwest::get(&self.jwks_uri)
                .await
                .map_err(|e| Error::ProviderError {
                    provider: self.jwks_uri.clone(),
                    detail: e.to_string(),
                })?;
        let keys: serde_json::Value =
            response
                .json()
                .await
                .map_err(|e| Error::ProviderError {
                    provider: self.jwks_uri.clone(),
                    detail: e.to_string(),
                })?;
        Ok(keys)
    }
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
        let cache = JwksCache::with_ttl(
            format!("{}/jwks", server.uri()),
            Duration::from_millis(1),
        );

        let _keys1 = cache.get_keys().await.expect("first call");

        // Wait for TTL to expire.
        tokio::time::sleep(Duration::from_millis(10)).await;

        let _keys2 = cache.get_keys().await.expect("second call after expiry");
        // wiremock's `expect(2)` verifies exactly 2 requests
    }
}
