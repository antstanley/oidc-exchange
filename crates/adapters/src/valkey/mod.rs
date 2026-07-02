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

/// The `COUNT` hint, in keys per page, passed to every `SCAN` issued by `cleanup_expired_sessions`.
/// This only advises the server on page size; it does not bound the total number of keys
/// visited (the client keeps paging until the cursor returns to 0).
const SCAN_BATCH_COUNT: u32 = 256;

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

        // Capture DEL's return count: it is 0 when the key was already gone (already
        // revoked or naturally expired) and 1 when this call actually removed it. Only
        // decrement the counter on an actual delete, so a repeated or already-expired
        // revoke does not double-decrement.
        let deleted_count: u64 = self.client.del(&key).await.map_err(|e| Error::StoreError {
            detail: e.to_string(),
        })?;
        assert!(
            deleted_count <= 1,
            "DEL of a single key must return 0 or 1, got {deleted_count}"
        );

        if deleted_count == 1 {
            let active_sessions_key = self.active_sessions_key();
            let counter: i64 =
                self.client
                    .decr(&active_sessions_key)
                    .await
                    .map_err(|e| Error::StoreError {
                        detail: e.to_string(),
                    })?;
            assert!(
                counter >= 0,
                "active_sessions counter must not go negative after a decrement, got {counter}"
            );
        }

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
        // Read the maintained counter rather than DBSIZE (which would count every key in
        // the database, including the user_sessions index sets and any keys outside
        // key_prefix). A missing key (nothing ever stored under this prefix) reads as 0
        // rather than erroring.
        let active_sessions_key = self.active_sessions_key();
        assert!(
            !active_sessions_key.is_empty(),
            "active_sessions key must not be empty"
        );

        let count: Option<u64> =
            self.client
                .get(&active_sessions_key)
                .await
                .map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })?;

        let result = count.unwrap_or(0);
        assert!(
            count.is_some() || result == 0,
            "a missing active_sessions key must report 0, not an arbitrary count"
        );
        Ok(result)
    }

    #[instrument(skip(self))]
    async fn cleanup_expired_sessions(&self) -> Result<u64> {
        use futures::stream::TryStreamExt;

        // Pass 1: prune dead members out of every `{prefix}user_sessions:*` index set. A
        // member is dead when its `{prefix}session:{hash}` key has already expired (or was
        // otherwise removed) server-side, so natural TTL expiry never shrinks the set on its
        // own — this pass is what reaps them.
        let user_sessions_pattern = format!("{}user_sessions:*", self.key_prefix);
        let set_keys: Vec<Key> = self
            .client
            .scan_buffered(user_sessions_pattern.as_str(), Some(SCAN_BATCH_COUNT), None)
            .try_collect()
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

        let mut removed_members: u64 = 0;
        let mut members_scanned: u64 = 0;

        for set_key in set_keys {
            let set_key = set_key.as_str_lossy().into_owned();
            let members: Vec<String> =
                self.client
                    .smembers(&set_key)
                    .await
                    .map_err(|e| Error::StoreError {
                        detail: e.to_string(),
                    })?;
            members_scanned += members.len() as u64;

            let mut dead_members = Vec::new();
            for member in &members {
                let session_key = self.session_key(member);
                let exists: bool =
                    self.client
                        .exists(&session_key)
                        .await
                        .map_err(|e| Error::StoreError {
                            detail: e.to_string(),
                        })?;
                if !exists {
                    dead_members.push(member.clone());
                }
            }

            if !dead_members.is_empty() {
                let dead_count = dead_members.len() as u64;
                let srem_count: u64 =
                    self.client
                        .srem(&set_key, dead_members)
                        .await
                        .map_err(|e| Error::StoreError {
                            detail: e.to_string(),
                        })?;
                assert!(
                    srem_count <= dead_count,
                    "SREM must not remove more members than were identified as dead, \
                     removed={srem_count} dead={dead_count}"
                );
                removed_members += srem_count;
            }

            // Delete the set once it holds no members, so an idle user's index set does not
            // linger forever as an empty key.
            let remaining: u64 =
                self.client
                    .scard(&set_key)
                    .await
                    .map_err(|e| Error::StoreError {
                        detail: e.to_string(),
                    })?;
            if remaining == 0 {
                self.client
                    .del::<(), _>(&set_key)
                    .await
                    .map_err(|e| Error::StoreError {
                        detail: e.to_string(),
                    })?;
            }
        }

        assert!(
            removed_members <= members_scanned,
            "the returned removed-count must not exceed the total members scanned across \
             all user_sessions sets, removed={removed_members} scanned={members_scanned}"
        );

        // Pass 2: reconcile the counter. INCR (on store) and DECR (on explicit revoke) never
        // account for natural TTL expiry, so the counter can only drift upward between
        // cleanups; recompute it here from a live SCAN of `{prefix}session:*` and SET it to
        // that exact count.
        let session_pattern = format!("{}session:*", self.key_prefix);
        let live_keys: Vec<Key> = self
            .client
            .scan_buffered(session_pattern.as_str(), Some(SCAN_BATCH_COUNT), None)
            .try_collect()
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;
        let live_count = live_keys.len() as u64;

        let active_sessions_key = self.active_sessions_key();
        self.client
            .set::<(), _, _>(&active_sessions_key, live_count, None, None, false)
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

        let reconciled: Option<u64> =
            self.client
                .get(&active_sessions_key)
                .await
                .map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })?;
        assert_eq!(
            reconciled.unwrap_or(0),
            live_count,
            "the reconciled active_sessions counter must equal the counted live session keys"
        );

        Ok(removed_members)
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

        // Delete every session key the user set names, then DECR the counter by exactly
        // the number of keys that actually existed (a stale/already-expired member in the
        // set must not decrement the counter). A single multi-key DEL both avoids an
        // unbounded round-trip loop and returns the live-key count directly.
        let deleted_count: u64 = if token_hashes.is_empty() {
            0
        } else {
            let keys: Vec<String> = token_hashes
                .iter()
                .map(|token_hash| self.session_key(token_hash))
                .collect();
            self.client.del(keys).await.map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?
        };
        assert!(
            deleted_count <= token_hashes.len() as u64,
            "deleted session-key count must not exceed the number of member hashes read, \
             got deleted={deleted_count} members={}",
            token_hashes.len()
        );

        if deleted_count > 0 {
            let active_sessions_key = self.active_sessions_key();
            let deleted_count_i64 =
                i64::try_from(deleted_count).map_err(|_| Error::StoreError {
                    detail: format!(
                        "deleted session-key count {deleted_count} overflows i64 for DECRBY"
                    ),
                })?;
            let counter: i64 = self
                .client
                .decr_by(&active_sessions_key, deleted_count_i64)
                .await
                .map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })?;
            assert!(
                counter >= 0,
                "active_sessions counter must not go negative after a decrement, got {counter}"
            );
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

    #[tokio::test]
    #[ignore] // Requires a local Valkey: docker run -p 6379:6379 valkey/valkey:8-alpine
    async fn count_active_sessions_on_untouched_prefix_returns_zero() {
        let repo = create_test_repo().await;

        let count = repo
            .count_active_sessions()
            .await
            .expect("count_active_sessions on a prefix with no counter key");

        assert_eq!(
            count, 0,
            "a prefix nothing has ever been stored under should report 0, not error"
        );
        let counter_exists: bool = repo
            .client
            .exists(&repo.active_sessions_key())
            .await
            .expect("exists on counter key");
        assert!(
            !counter_exists,
            "an untouched prefix must not have created a counter key as a side effect"
        );
    }

    #[tokio::test]
    #[ignore] // Requires a local Valkey: docker run -p 6379:6379 valkey/valkey:8-alpine
    async fn revoke_session_decrements_counter_exactly_once() {
        let repo = create_test_repo().await;
        let session = sample_session("usr_revoke", "hash_revoke", 3600);

        repo.store_refresh_token(&session)
            .await
            .expect("store_refresh_token");
        assert_eq!(
            repo.count_active_sessions()
                .await
                .expect("count after store"),
            1,
            "counter should read 1 after one store"
        );

        repo.revoke_session(&session.refresh_token_hash)
            .await
            .expect("first revoke_session");
        let count_after_first_revoke = repo
            .count_active_sessions()
            .await
            .expect("count after first revoke");
        assert_eq!(
            count_after_first_revoke, 0,
            "counter should drop from 1 to 0 after the session is revoked"
        );

        let key = repo.session_key(&session.refresh_token_hash);
        let exists: bool = repo
            .client
            .exists(&key)
            .await
            .expect("exists on session hash after revoke");
        assert!(!exists, "revoked session hash key must be gone");

        // A second revoke of the same (already-gone) token must not double-decrement: DEL
        // returns 0 because the key no longer exists, so the counter must stay at 0.
        repo.revoke_session(&session.refresh_token_hash)
            .await
            .expect("second revoke_session on an already-revoked token");
        let count_after_second_revoke = repo
            .count_active_sessions()
            .await
            .expect("count after second revoke");
        assert_eq!(
            count_after_second_revoke, 0,
            "a repeated revoke of an already-deleted session must not decrement the counter again"
        );
    }

    #[tokio::test]
    #[ignore] // Requires a local Valkey: docker run -p 6379:6379 valkey/valkey:8-alpine
    async fn revoke_all_user_sessions_decrements_by_live_key_count() {
        let repo = create_test_repo().await;
        let user_id = "usr_revoke_all";

        let session_a = sample_session(user_id, "hash_all_a", 3600);
        let session_b = sample_session(user_id, "hash_all_b", 3600);
        let session_c = sample_session(user_id, "hash_all_c", 3600);
        repo.store_refresh_token(&session_a)
            .await
            .expect("store session a");
        repo.store_refresh_token(&session_b)
            .await
            .expect("store session b");
        repo.store_refresh_token(&session_c)
            .await
            .expect("store session c");
        assert_eq!(
            repo.count_active_sessions()
                .await
                .expect("count after 3 stores"),
            3,
            "counter should read 3 after three stores for the same user"
        );

        // Revoke one session individually first. revoke_session also SREMs the user set, so
        // after this the set holds exactly the two remaining live members (hash_all_b,
        // hash_all_c) — not all three with one stale.
        repo.revoke_session(&session_a.refresh_token_hash)
            .await
            .expect("revoke session a individually");
        assert_eq!(
            repo.count_active_sessions()
                .await
                .expect("count after single revoke"),
            2,
            "counter should read 2 after individually revoking one of three sessions"
        );

        // Delete session_b's hash directly through the client, bypassing revoke_session, so
        // the counter is NOT decremented for it. This leaves the user set naming a member
        // (hash_all_b) whose session key no longer exists — a stale member — while the
        // counter still reads 2 even though only one live key (session_c) remains. This
        // reproduces the exact drift revoke_all_user_sessions must handle: it must DECRBY
        // the counter by the number of keys its own DEL actually removed (1, for
        // session_c), not by the number of members read from the set (2).
        let deleted_directly: u64 = repo
            .client
            .del(repo.session_key("hash_all_b"))
            .await
            .expect("delete session_b's hash directly, bypassing revoke_session");
        assert_eq!(
            deleted_directly, 1,
            "the direct DEL of session_b's hash must have removed exactly one key"
        );
        assert_eq!(
            repo.count_active_sessions()
                .await
                .expect("count before revoke_all_user_sessions"),
            2,
            "the counter must still read 2: the direct DEL above bypassed revoke_session, so \
             it never decremented the counter, leaving it stale relative to the one truly \
             live key (session_c)"
        );

        repo.revoke_all_user_sessions(user_id)
            .await
            .expect("revoke_all_user_sessions");

        let count_after_revoke_all = repo
            .count_active_sessions()
            .await
            .expect("count after revoke_all_user_sessions");
        assert_eq!(
            count_after_revoke_all, 1,
            "counter should drop from 2 to 1: revoke_all_user_sessions read two members from \
             the set (hash_all_b, hash_all_c) but its DEL only actually removed one live key \
             (session_c, since session_b's hash was already gone), so it must DECRBY 1 — the \
             actual delete count — not DECRBY 2, the set-membership count"
        );

        let user_set_key = repo.user_sessions_key(user_id);
        let set_exists: bool = repo
            .client
            .exists(&user_set_key)
            .await
            .expect("exists on user set after revoke_all_user_sessions");
        assert!(
            !set_exists,
            "the user set itself must be deleted by revoke_all_user_sessions"
        );

        for hash in ["hash_all_a", "hash_all_b", "hash_all_c"] {
            let key = repo.session_key(hash);
            let exists: bool = repo
                .client
                .exists(&key)
                .await
                .expect("exists on session hash after revoke_all_user_sessions");
            assert!(
                !exists,
                "session hash {hash} must be gone after revoke_all_user_sessions"
            );
        }
    }

    #[tokio::test]
    #[ignore] // Requires a local Valkey: docker run -p 6379:6379 valkey/valkey:8-alpine
    async fn cleanup_expired_sessions_prunes_only_member_deletes_set_and_resets_counter() {
        let repo = create_test_repo().await;
        let user_id = "usr_cleanup_solo";

        // A 2s TTL (rather than the theoretical 1s floor) tolerates the sub-second delay
        // between computing `expires_at` here and `store_refresh_token` truncating it down
        // to whole seconds via `num_seconds()`, so the write is never spuriously rejected.
        let session = sample_session(user_id, "hash_cleanup_solo", 2);
        repo.store_refresh_token(&session)
            .await
            .expect("store short-lived session");

        let user_set_key = repo.user_sessions_key(user_id);
        // Extend the set's own TTL well beyond the session's short TTL so the set (with its
        // now-stale sole member) is still present when cleanup runs, decoupling this test
        // from a race against Valkey's own TTL expiry of the set itself.
        let _: bool = repo
            .client
            .expire(&user_set_key, 10, None)
            .await
            .expect("extend user set TTL for deterministic test setup");

        assert_eq!(
            repo.count_active_sessions()
                .await
                .expect("count after store"),
            1,
            "counter should read 1 after one store"
        );

        tokio::time::sleep(std::time::Duration::from_millis(2500)).await;

        let key = repo.session_key(&session.refresh_token_hash);
        let exists: bool = repo
            .client
            .exists(&key)
            .await
            .expect("exists on session hash after TTL expiry");
        assert!(!exists, "the short-TTL session hash must have expired");

        let set_exists_before: bool = repo
            .client
            .exists(&user_set_key)
            .await
            .expect("exists on user set before cleanup");
        assert!(
            set_exists_before,
            "the extended set TTL must have kept the set (with its stale member) alive"
        );

        // Natural TTL expiry never decrements the counter: it still over-reports the one
        // session that has since expired.
        assert_eq!(
            repo.count_active_sessions()
                .await
                .expect("count before cleanup"),
            1,
            "the counter must still read 1, drifted above the zero truly live sessions"
        );

        let removed = repo
            .cleanup_expired_sessions()
            .await
            .expect("cleanup_expired_sessions");
        assert_eq!(
            removed, 1,
            "cleanup should prune exactly the one stale user_sessions member"
        );

        let set_exists_after: bool = repo
            .client
            .exists(&user_set_key)
            .await
            .expect("exists on user set after cleanup");
        assert!(
            !set_exists_after,
            "the now-empty user_sessions set must be deleted by cleanup"
        );

        let counter_after = repo
            .count_active_sessions()
            .await
            .expect("count after cleanup");
        assert_eq!(
            counter_after, 0,
            "the reconciled counter must equal the zero live session keys, resetting the drift"
        );
    }

    #[tokio::test]
    #[ignore] // Requires a local Valkey: docker run -p 6379:6379 valkey/valkey:8-alpine
    async fn cleanup_expired_sessions_with_no_dead_members_returns_zero_and_matches_live_count() {
        let repo = create_test_repo().await;
        let user_id = "usr_cleanup_clean";

        let session = sample_session(user_id, "hash_cleanup_clean", 3600);
        repo.store_refresh_token(&session)
            .await
            .expect("store live session");

        let removed = repo
            .cleanup_expired_sessions()
            .await
            .expect("cleanup_expired_sessions over a clean prefix");
        assert_eq!(
            removed, 0,
            "cleanup over a prefix with no dead members must return 0"
        );

        let user_set_key = repo.user_sessions_key(user_id);
        let members: Vec<String> = repo
            .client
            .smembers(&user_set_key)
            .await
            .expect("smembers on user set after cleanup");
        assert!(
            members.contains(&session.refresh_token_hash),
            "the live member must be untouched by cleanup"
        );

        let counter_after = repo
            .count_active_sessions()
            .await
            .expect("count after cleanup");
        assert_eq!(
            counter_after, 1,
            "the counter must equal the live-key count (1) after cleanup with no drift"
        );
    }
}
