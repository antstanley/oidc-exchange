use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fred::prelude::*;
use fred::types::ExpireOptions;
use oidc_exchange_core::domain::Session;
use oidc_exchange_core::error::{Error, Result};
use oidc_exchange_core::ports::SessionRepository;
use tracing::instrument;

/// The minimum TTL, in seconds, a session may be stored with. `store_refresh_token` computes
/// `ttl_seconds` from `expires_at - now`; a value below this floor means `expires_at` is not
/// strictly in the future, so the write is rejected before any key is created (no immortal or
/// already-expired Valkey key is ever written).
const SESSION_TTL_SECONDS_MIN: i64 = 1;

pub struct ValkeySessionRepository {
    client: fred::clients::Client,
    key_prefix: String,
}

impl ValkeySessionRepository {
    pub async fn new(
        url: &str,
        key_prefix: String,
    ) -> std::result::Result<Self, Box<dyn std::error::Error>> {
        let config = Config::from_url(url)?;
        let client = fred::clients::Client::new(config, None, None, None);
        client.init().await?;
        Ok(Self { client, key_prefix })
    }

    fn session_key(&self, token_hash: &str) -> String {
        format!("{}session:{}", self.key_prefix, token_hash)
    }

    fn user_sessions_key(&self, user_id: &str) -> String {
        format!("{}user_sessions:{}", self.key_prefix, user_id)
    }

    fn active_sessions_key(&self) -> String {
        format!("{}active_sessions", self.key_prefix)
    }
}

#[async_trait]
impl SessionRepository for ValkeySessionRepository {
    #[instrument(skip(self))]
    async fn store_refresh_token(&self, session: &Session) -> Result<()> {
        // Compute TTL from expires_at first and reject before issuing any write: a session
        // whose expires_at is not strictly in the future would otherwise leave a TTL-less (or
        // already-dead) hash behind.
        let ttl_seconds = (session.expires_at - Utc::now()).num_seconds();
        if ttl_seconds < SESSION_TTL_SECONDS_MIN {
            return Err(Error::StoreError {
                detail: format!(
                    "refusing to store session with non-future expires_at (ttl_seconds={ttl_seconds})"
                ),
            });
        }
        debug_assert!(ttl_seconds >= SESSION_TTL_SECONDS_MIN);

        let key = self.session_key(&session.refresh_token_hash);
        let user_sessions_key = self.user_sessions_key(&session.user_id);
        let active_sessions_key = self.active_sessions_key();

        assert!(!key.is_empty(), "session key must not be empty");
        assert!(
            !user_sessions_key.is_empty(),
            "user_sessions key must not be empty"
        );

        let fields: Vec<(&str, String)> = vec![
            ("user_id", session.user_id.clone()),
            ("refresh_token_hash", session.refresh_token_hash.clone()),
            ("provider", session.provider.clone()),
            ("expires_at", session.expires_at.to_rfc3339()),
            ("device_id", session.device_id.clone().unwrap_or_default()),
            ("user_agent", session.user_agent.clone().unwrap_or_default()),
            ("ip_address", session.ip_address.clone().unwrap_or_default()),
            ("created_at", session.created_at.to_rfc3339()),
        ];

        // Batch the hash write, both TTLs, the user-set membership, and the counter INCR into
        // a single pipeline so a crash between commands can never leave a TTL-less hash, a
        // half-written index, or a missed count.
        let pipeline = self.client.pipeline();
        let _: () = pipeline
            .hset(&key, fields)
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;
        let _: () = pipeline
            .expire(&key, ttl_seconds, None)
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;
        let _: () = pipeline
            .sadd(&user_sessions_key, &session.refresh_token_hash)
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;
        // Bootstrap the set's TTL on its first-ever member (NX: only when the set currently
        // has no expiry — SADD above may have just created it with none), then only-extend on
        // every write (GT: Valkey treats "no expiry" as infinite for GT, so GT alone would
        // never TTL a brand-new set). Together these give "TTL bumped to the greatest member
        // expiry" without a concurrent shorter-lived write ever shortening the set's life.
        let _: () = pipeline
            .expire(&user_sessions_key, ttl_seconds, Some(ExpireOptions::NX))
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;
        let _: () = pipeline
            .expire(&user_sessions_key, ttl_seconds, Some(ExpireOptions::GT))
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;
        let _: () = pipeline
            .incr(&active_sessions_key)
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

        pipeline.all::<()>().await.map_err(|e| Error::StoreError {
            detail: e.to_string(),
        })?;

        Ok(())
    }

    #[instrument(skip(self))]
    async fn get_session_by_refresh_token(&self, token_hash: &str) -> Result<Option<Session>> {
        let key = self.session_key(token_hash);

        let values: std::collections::HashMap<String, String> = self
            .client
            .hgetall(&key)
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

        if values.is_empty() {
            return Ok(None);
        }

        let get_field = |name: &str| -> Result<String> {
            values.get(name).cloned().ok_or_else(|| Error::StoreError {
                detail: format!("missing field: {}", name),
            })
        };

        let parse_dt = |s: &str| -> Result<DateTime<Utc>> {
            DateTime::parse_from_rfc3339(s)
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })
        };

        let device_id =
            values
                .get("device_id")
                .and_then(|v| if v.is_empty() { None } else { Some(v.clone()) });
        let user_agent =
            values.get("user_agent").and_then(
                |v| {
                    if v.is_empty() {
                        None
                    } else {
                        Some(v.clone())
                    }
                },
            );
        let ip_address =
            values.get("ip_address").and_then(
                |v| {
                    if v.is_empty() {
                        None
                    } else {
                        Some(v.clone())
                    }
                },
            );

        let expires_at_str = get_field("expires_at")?;
        let created_at_str = get_field("created_at")?;

        Ok(Some(Session {
            user_id: get_field("user_id")?,
            refresh_token_hash: get_field("refresh_token_hash")?,
            provider: get_field("provider")?,
            expires_at: parse_dt(&expires_at_str)?,
            device_id,
            user_agent,
            ip_address,
            created_at: parse_dt(&created_at_str)?,
        }))
    }

    #[instrument(skip(self))]
    async fn revoke_session(&self, token_hash: &str) -> Result<()> {
        let key = self.session_key(token_hash);

        // Get user_id before deleting so we can clean up the user set
        let user_id: Option<String> =
            self.client
                .hget(&key, "user_id")
                .await
                .map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })?;

        self.client
            .del::<(), _>(&key)
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

        if let Some(user_id) = user_id {
            self.client
                .srem::<(), _, _>(self.user_sessions_key(&user_id), token_hash)
                .await
                .map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })?;
        }

        Ok(())
    }

    #[instrument(skip(self))]
    async fn count_active_sessions(&self) -> Result<u64> {
        // Count all session keys. Valkey sessions have TTL set, so existing keys
        // are active by definition (expired keys are removed automatically).
        let count: u64 = self.client.dbsize().await.map_err(|e| Error::StoreError {
            detail: e.to_string(),
        })?;
        // dbsize returns total keys including user_sessions sets.
        // For an approximate count, this is acceptable. For exact counts,
        // a scan with prefix matching would be needed.
        Ok(count)
    }

    #[instrument(skip(self))]
    async fn cleanup_expired_sessions(&self) -> Result<u64> {
        // Valkey/Redis TTL handles expiration automatically.
        // Keys with TTL are deleted by the server, so this is a no-op.
        Ok(0)
    }

    #[instrument(skip(self))]
    async fn revoke_all_user_sessions(&self, user_id: &str) -> Result<()> {
        let user_set_key = self.user_sessions_key(user_id);

        let token_hashes: Vec<String> =
            self.client
                .smembers(&user_set_key)
                .await
                .map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })?;

        for token_hash in &token_hashes {
            let key = self.session_key(token_hash);
            self.client
                .del::<(), _>(&key)
                .await
                .map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })?;
        }

        self.client
            .del::<(), _>(&user_set_key)
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Integration tests (require a live Valkey/Redis)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Env var carrying the Valkey/Redis URL used by the `#[ignore]`d tests below; defaults to
    /// a local server, matching how the DynamoDB Local tests hardcode their endpoint.
    fn valkey_test_url() -> String {
        std::env::var("VALKEY_TEST_URL").unwrap_or_else(|_| "redis://localhost:6379".to_string())
    }

    /// Builds a repository against a unique `key_prefix` per call so concurrent/successive test
    /// runs never collide and self-clean (a fresh prefix has no pre-existing keys).
    async fn create_test_repo() -> ValkeySessionRepository {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let prefix = format!(
            "oidc_exchange_test:{}:{}:",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock before epoch")
                .as_nanos(),
        );
        let prefix = format!("{prefix}{n}:");

        ValkeySessionRepository::new(&valkey_test_url(), prefix)
            .await
            .expect("connect to local Valkey (VALKEY_TEST_URL or redis://localhost:6379)")
    }

    fn sample_session(user_id: &str, hash: &str, ttl_seconds: i64) -> Session {
        let now = Utc::now();
        Session {
            user_id: user_id.to_string(),
            refresh_token_hash: hash.to_string(),
            provider: "google".to_string(),
            expires_at: now + chrono::Duration::seconds(ttl_seconds),
            device_id: Some("device-1".to_string()),
            user_agent: Some("test-agent".to_string()),
            ip_address: Some("10.0.0.1".to_string()),
            created_at: now,
        }
    }

    #[tokio::test]
    #[ignore] // Requires a local Valkey: docker run -p 6379:6379 valkey/valkey:8-alpine
    async fn store_refresh_token_writes_ttld_hash_set_member_and_counter() {
        let repo = create_test_repo().await;
        let session = sample_session("usr_1", "hash_abc", 3600);

        repo.store_refresh_token(&session)
            .await
            .expect("store_refresh_token");

        let key = repo.session_key(&session.refresh_token_hash);
        let user_set_key = repo.user_sessions_key(&session.user_id);
        let counter_key = repo.active_sessions_key();

        let hash_ttl: i64 = repo.client.ttl(&key).await.expect("ttl on session hash");
        assert!(
            hash_ttl > 0 && hash_ttl <= 3600,
            "session hash TTL should be positive and bounded by the stored TTL, got {hash_ttl}"
        );

        let members: Vec<String> = repo
            .client
            .smembers(&user_set_key)
            .await
            .expect("smembers on user set");
        assert!(
            members.contains(&session.refresh_token_hash),
            "user set should contain the stored session's hash"
        );

        let set_ttl: i64 = repo
            .client
            .ttl(&user_set_key)
            .await
            .expect("ttl on user set");
        assert!(
            set_ttl > 0 && set_ttl <= 3600,
            "user set TTL should be bumped to a positive value, got {set_ttl}"
        );

        let counter: u64 = repo.client.get(&counter_key).await.expect("get counter");
        assert_eq!(counter, 1, "counter should read 1 after one store");
    }

    #[tokio::test]
    #[ignore] // Requires a local Valkey: docker run -p 6379:6379 valkey/valkey:8-alpine
    async fn store_refresh_token_set_ttl_only_extends() {
        let repo = create_test_repo().await;
        let user_id = "usr_2";

        let long_session = sample_session(user_id, "hash_long", 3600);
        repo.store_refresh_token(&long_session)
            .await
            .expect("store long-lived session");

        let user_set_key = repo.user_sessions_key(user_id);
        let ttl_after_long: i64 = repo
            .client
            .ttl(&user_set_key)
            .await
            .expect("ttl after long-lived write");

        // A second store for the same user with a much shorter TTL must not shorten the
        // already-longer set TTL (EXPIRE ... GT is only-extend).
        let short_session = sample_session(user_id, "hash_short", 5);
        repo.store_refresh_token(&short_session)
            .await
            .expect("store short-lived session");

        let ttl_after_short: i64 = repo
            .client
            .ttl(&user_set_key)
            .await
            .expect("ttl after short-lived write");

        assert!(
            ttl_after_short > 5,
            "GT bump must not shorten the set TTL down to the shorter write's TTL, got {ttl_after_short}"
        );
        assert!(
            ttl_after_short <= ttl_after_long,
            "set TTL should never exceed the longest TTL ever applied (allowing for clock drift), \
             got before={ttl_after_long} after={ttl_after_short}"
        );

        let counter: u64 = repo
            .client
            .get(&repo.active_sessions_key())
            .await
            .expect("get counter");
        assert_eq!(counter, 2, "counter should read 2 after two stores");
    }

    #[tokio::test]
    #[ignore] // Requires a local Valkey: docker run -p 6379:6379 valkey/valkey:8-alpine
    async fn store_refresh_token_rejects_non_future_expiry() {
        let repo = create_test_repo().await;

        // expires_at at "now" (ttl_seconds == 0, below the floor of 1).
        let at_now = sample_session("usr_3", "hash_at_now", 0);
        let err = repo
            .store_refresh_token(&at_now)
            .await
            .expect_err("expires_at at now should be rejected");
        assert!(matches!(err, Error::StoreError { .. }));

        // expires_at in the past.
        let in_past = sample_session("usr_3", "hash_in_past", -60);
        let err = repo
            .store_refresh_token(&in_past)
            .await
            .expect_err("expires_at in the past should be rejected");
        assert!(matches!(err, Error::StoreError { .. }));

        let key = repo.session_key(&at_now.refresh_token_hash);
        let exists: bool = repo
            .client
            .exists(&key)
            .await
            .expect("exists on session hash");
        assert!(!exists, "rejected session must not create a hash key");

        let counter_key = repo.active_sessions_key();
        let counter_exists: bool = repo
            .client
            .exists(&counter_key)
            .await
            .expect("exists on counter key");
        assert!(
            !counter_exists,
            "rejected session must not increment (or create) the counter"
        );
    }
}
