use async_trait::async_trait;
use chrono::{DateTime, Utc};
use heed::types::{Bytes, Str};
use heed::{Database, Env, EnvOpenOptions};
use oidc_exchange_core::domain::Session;
use oidc_exchange_core::error::Error;
use oidc_exchange_core::ports::SessionRepository;
use std::fs;
use tracing::instrument;

/// LMDB-backed session repository using the `heed` crate.
///
/// Three named databases are maintained:
/// - `sessions`: `token_hash -> JSON(Session)`
/// - `user_sessions`: `"{user_id}:{token_hash}" -> ""` (secondary index)
/// - `single_use`: `digest_key -> RFC3339(expires_at)` for single-use records
pub struct LmdbSessionRepository {
    env: Env,
    sessions: Database<Str, Bytes>,
    user_sessions: Database<Str, Str>,
    /// Single-use records (nonces, assertion-replay markers), keyed by namespaced
    /// digest. A separate named database keeps their key space disjoint from session
    /// token hashes.
    single_use: Database<Str, Str>,
}

impl LmdbSessionRepository {
    /// Opens (or creates) an LMDB environment at `path` with the given max size.
    pub fn new(path: &str, max_size_mb: u64) -> Result<Self, Box<dyn std::error::Error>> {
        fs::create_dir_all(path)?;

        let env = unsafe {
            EnvOpenOptions::new()
                .max_dbs(3)
                .map_size((max_size_mb * 1024 * 1024) as usize)
                .open(path)?
        };

        let mut wtxn = env.write_txn()?;
        let sessions: Database<Str, Bytes> = env.create_database(&mut wtxn, Some("sessions"))?;
        let user_sessions: Database<Str, Str> =
            env.create_database(&mut wtxn, Some("user_sessions"))?;
        let single_use: Database<Str, Str> = env.create_database(&mut wtxn, Some("single_use"))?;
        wtxn.commit()?;

        Ok(Self {
            env,
            sessions,
            user_sessions,
            single_use,
        })
    }

    /// Build the composite key used in the `user_sessions` index.
    fn user_session_key(user_id: &str, token_hash: &str) -> String {
        format!("{user_id}:{token_hash}")
    }
}

#[async_trait]
impl SessionRepository for LmdbSessionRepository {
    #[instrument(skip(self, session), fields(token_hash = %session.refresh_token_hash, user_id = %session.user_id))]
    async fn store_refresh_token(
        &self,
        session: &Session,
    ) -> oidc_exchange_core::error::Result<()> {
        let env = self.env.clone();
        let sessions_db = self.sessions;
        let user_sessions_db = self.user_sessions;
        let session = session.clone();

        tokio::task::spawn_blocking(move || {
            let json = serde_json::to_vec(&session).map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

            let mut wtxn = env.write_txn().map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

            sessions_db
                .put(&mut wtxn, &session.refresh_token_hash, &json)
                .map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })?;

            let index_key = LmdbSessionRepository::user_session_key(
                &session.user_id,
                &session.refresh_token_hash,
            );
            user_sessions_db
                .put(&mut wtxn, &index_key, "")
                .map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })?;

            wtxn.commit().map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

            Ok(())
        })
        .await
        .map_err(|e| Error::StoreError {
            detail: e.to_string(),
        })?
    }

    #[instrument(skip(self))]
    async fn get_session_by_refresh_token(
        &self,
        token_hash: &str,
    ) -> oidc_exchange_core::error::Result<Option<Session>> {
        let env = self.env.clone();
        let sessions_db = self.sessions;
        let token_hash = token_hash.to_owned();

        tokio::task::spawn_blocking(move || {
            let rtxn = env.read_txn().map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

            let maybe_bytes =
                sessions_db
                    .get(&rtxn, &token_hash)
                    .map_err(|e| Error::StoreError {
                        detail: e.to_string(),
                    })?;

            match maybe_bytes {
                Some(bytes) => {
                    let session: Session =
                        serde_json::from_slice(bytes).map_err(|e| Error::StoreError {
                            detail: e.to_string(),
                        })?;
                    Ok(Some(session))
                }
                None => Ok(None),
            }
        })
        .await
        .map_err(|e| Error::StoreError {
            detail: e.to_string(),
        })?
    }

    #[instrument(skip(self))]
    async fn revoke_session(&self, token_hash: &str) -> oidc_exchange_core::error::Result<()> {
        let env = self.env.clone();
        let sessions_db = self.sessions;
        let user_sessions_db = self.user_sessions;
        let token_hash = token_hash.to_owned();

        tokio::task::spawn_blocking(move || {
            // First, read the session to get the user_id for index cleanup.
            let rtxn = env.read_txn().map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

            let maybe_bytes =
                sessions_db
                    .get(&rtxn, &*token_hash)
                    .map_err(|e| Error::StoreError {
                        detail: e.to_string(),
                    })?;

            let user_id = match maybe_bytes {
                Some(bytes) => {
                    let session: Session =
                        serde_json::from_slice(bytes).map_err(|e| Error::StoreError {
                            detail: e.to_string(),
                        })?;
                    Some(session.user_id)
                }
                None => None,
            };
            drop(rtxn);

            let mut wtxn = env.write_txn().map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

            // Delete from sessions db.
            sessions_db
                .delete(&mut wtxn, &*token_hash)
                .map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })?;

            // Delete from user_sessions index if we found the user_id.
            if let Some(uid) = user_id {
                let index_key = LmdbSessionRepository::user_session_key(&uid, &token_hash);
                user_sessions_db
                    .delete(&mut wtxn, &index_key)
                    .map_err(|e| Error::StoreError {
                        detail: e.to_string(),
                    })?;
            }

            wtxn.commit().map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

            Ok(())
        })
        .await
        .map_err(|e| Error::StoreError {
            detail: e.to_string(),
        })?
    }

    #[instrument(skip(self))]
    async fn count_active_sessions(&self) -> oidc_exchange_core::error::Result<u64> {
        let env = self.env.clone();
        let sessions_db = self.sessions;

        tokio::task::spawn_blocking(move || {
            let now = Utc::now();
            let rtxn = env.read_txn().map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

            let mut count: u64 = 0;
            let iter = sessions_db.iter(&rtxn).map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

            for result in iter {
                let (_key, bytes) = result.map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })?;
                if let Ok(session) = serde_json::from_slice::<Session>(bytes) {
                    if session.expires_at > now {
                        count += 1;
                    }
                }
            }

            Ok(count)
        })
        .await
        .map_err(|e| Error::StoreError {
            detail: e.to_string(),
        })?
    }

    #[instrument(skip(self))]
    async fn cleanup_expired_sessions(&self) -> oidc_exchange_core::error::Result<u64> {
        let env = self.env.clone();
        let sessions_db = self.sessions;
        let user_sessions_db = self.user_sessions;
        let single_use_db = self.single_use;

        tokio::task::spawn_blocking(move || {
            let now = Utc::now();

            let rtxn = env.read_txn().map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

            let mut to_delete: Vec<(String, String)> = Vec::new(); // (token_hash, user_id)
            let iter = sessions_db.iter(&rtxn).map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

            for result in iter {
                let (key, bytes) = result.map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })?;
                if let Ok(session) = serde_json::from_slice::<Session>(bytes) {
                    if session.expires_at <= now {
                        to_delete.push((key.to_owned(), session.user_id.clone()));
                    }
                }
            }

            // Collect expired single-use records too: LMDB has no native expiry, so the
            // sweep is their only space reclamation. The claim operations already treat
            // an expired record as absent, so a skipped sweep never affects correctness.
            let mut single_use_to_delete: Vec<String> = Vec::new();
            let single_use_iter = single_use_db.iter(&rtxn).map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;
            for result in single_use_iter {
                let (key, expires_at_str) = result.map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })?;
                match DateTime::parse_from_rfc3339(expires_at_str) {
                    Ok(expires_at) if expires_at.with_timezone(&Utc) <= now => {
                        single_use_to_delete.push(key.to_owned());
                    }
                    Ok(_) => {}
                    Err(e) => {
                        return Err(Error::StoreError {
                            detail: format!(
                                "single_use record has unparsable expiry (key withheld): {e}"
                            ),
                        });
                    }
                }
            }
            drop(rtxn);

            if to_delete.is_empty() && single_use_to_delete.is_empty() {
                return Ok(0);
            }

            let deleted = to_delete.len() as u64 + single_use_to_delete.len() as u64;
            let mut wtxn = env.write_txn().map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

            for (token_hash, user_id) in &to_delete {
                sessions_db
                    .delete(&mut wtxn, token_hash.as_str())
                    .map_err(|e| Error::StoreError {
                        detail: e.to_string(),
                    })?;

                let index_key = LmdbSessionRepository::user_session_key(user_id, token_hash);
                user_sessions_db
                    .delete(&mut wtxn, index_key.as_str())
                    .map_err(|e| Error::StoreError {
                        detail: e.to_string(),
                    })?;
            }

            for key in &single_use_to_delete {
                single_use_db
                    .delete(&mut wtxn, key.as_str())
                    .map_err(|e| Error::StoreError {
                        detail: e.to_string(),
                    })?;
            }

            wtxn.commit().map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

            debug_assert!(deleted > 0, "a non-empty sweep must report deletions");
            Ok(deleted)
        })
        .await
        .map_err(|e| Error::StoreError {
            detail: e.to_string(),
        })?
    }

    #[instrument(skip(self))]
    async fn revoke_all_user_sessions(
        &self,
        user_id: &str,
    ) -> oidc_exchange_core::error::Result<()> {
        let env = self.env.clone();
        let sessions_db = self.sessions;
        let user_sessions_db = self.user_sessions;
        let user_id = user_id.to_owned();

        tokio::task::spawn_blocking(move || {
            let prefix = format!("{user_id}:");

            // Collect all matching index keys and their token hashes.
            let rtxn = env.read_txn().map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

            let mut to_delete: Vec<(String, String)> = Vec::new(); // (index_key, token_hash)

            let iter =
                user_sessions_db
                    .prefix_iter(&rtxn, &prefix)
                    .map_err(|e| Error::StoreError {
                        detail: e.to_string(),
                    })?;

            for result in iter {
                let (key, _val) = result.map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })?;
                // key is "user_id:token_hash"
                if let Some(token_hash) = key.strip_prefix(&prefix) {
                    to_delete.push((key.to_owned(), token_hash.to_owned()));
                }
            }
            drop(rtxn);

            if to_delete.is_empty() {
                return Ok(());
            }

            let mut wtxn = env.write_txn().map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

            for (index_key, token_hash) in &to_delete {
                // Delete from sessions db.
                sessions_db
                    .delete(&mut wtxn, token_hash.as_str())
                    .map_err(|e| Error::StoreError {
                        detail: e.to_string(),
                    })?;

                // Delete from user_sessions index.
                user_sessions_db
                    .delete(&mut wtxn, index_key.as_str())
                    .map_err(|e| Error::StoreError {
                        detail: e.to_string(),
                    })?;
            }

            wtxn.commit().map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

            Ok(())
        })
        .await
        .map_err(|e| Error::StoreError {
            detail: e.to_string(),
        })?
    }

    #[instrument(skip(self, key))]
    async fn put_single_use(
        &self,
        key: &str,
        expires_at: DateTime<Utc>,
    ) -> oidc_exchange_core::error::Result<bool> {
        assert!(!key.is_empty(), "single-use key must be non-empty");

        let env = self.env.clone();
        let single_use_db = self.single_use;
        let key = key.to_owned();
        let expires_at_str = expires_at.to_rfc3339();

        // One write transaction does the whole read-check-write: LMDB write transactions
        // are exclusive, so two racing claims of one key serialize and exactly one sees
        // an absent-or-expired predecessor.
        tokio::task::spawn_blocking(move || {
            let mut wtxn = env.write_txn().map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

            if let Some(existing) =
                single_use_db
                    .get(&wtxn, &key)
                    .map_err(|e| Error::StoreError {
                        detail: e.to_string(),
                    })?
            {
                let existing_expiry = DateTime::parse_from_rfc3339(existing)
                    .map_err(|e| Error::StoreError {
                        detail: format!("single_use record has unparsable expiry: {e}"),
                    })?
                    .with_timezone(&Utc);
                if existing_expiry > Utc::now() {
                    // A live record holds the key; abort the (read-only so far) txn.
                    return Ok(false);
                }
                // Expired-is-absent: fall through and overwrite the dead record.
                debug_assert!(existing_expiry <= Utc::now());
            }

            single_use_db
                .put(&mut wtxn, &key, &expires_at_str)
                .map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })?;
            wtxn.commit().map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

            Ok(true)
        })
        .await
        .map_err(|e| Error::StoreError {
            detail: e.to_string(),
        })?
    }

    #[instrument(skip(self, key))]
    async fn take_single_use(&self, key: &str) -> oidc_exchange_core::error::Result<bool> {
        assert!(!key.is_empty(), "single-use key must be non-empty");

        let env = self.env.clone();
        let single_use_db = self.single_use;
        let key = key.to_owned();

        tokio::task::spawn_blocking(move || {
            let mut wtxn = env.write_txn().map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

            let was_live = match single_use_db.get(&wtxn, &key) {
                Ok(Some(expires_at_str)) => {
                    let expires_at = DateTime::parse_from_rfc3339(expires_at_str)
                        .map_err(|e| Error::StoreError {
                            detail: format!("single_use record has unparsable expiry: {e}"),
                        })?
                        .with_timezone(&Utc);
                    // Delete whatever is present (live or expired): burning is the only
                    // path here, so reclaiming a dead record while reporting false is
                    // free space reclamation with no semantic difference to the caller.
                    single_use_db
                        .delete(&mut wtxn, &key)
                        .map_err(|e| Error::StoreError {
                            detail: e.to_string(),
                        })?;
                    expires_at > Utc::now()
                }
                Ok(None) => false,
                Err(e) => {
                    return Err(Error::StoreError {
                        detail: e.to_string(),
                    })
                }
            };

            // Commit even when nothing matched: an empty write transaction is cheap,
            // and branching on "was anything deleted" would buy nothing.
            wtxn.commit().map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

            Ok(was_live)
        })
        .await
        .map_err(|e| Error::StoreError {
            detail: e.to_string(),
        })?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oidc_exchange_core::domain::Session;
    use std::sync::Arc;

    async fn create_test_repo() -> LmdbSessionRepository {
        let dir = tempfile::tempdir().expect("tempdir");
        // Keep the TempDir alive by leaking: tests are short-lived processes and the
        // repo borrows nothing from it after open, but LMDB memory-maps the files.
        let path = dir.path().join("lmdb_test");
        let path_str = path.to_str().expect("utf8 path").to_string();
        std::mem::forget(dir);
        LmdbSessionRepository::new(&path_str, 16).expect("open lmdb env")
    }

    fn sample_session(user_id: &str, hash: &str, ttl_seconds: i64) -> Session {
        let now = Utc::now();
        Session {
            user_id: user_id.to_string(),
            refresh_token_hash: hash.to_string(),
            provider: "google".to_string(),
            expires_at: now + chrono::Duration::seconds(ttl_seconds),
            device_id: None,
            user_agent: None,
            ip_address: None,
            created_at: now,
        }
    }

    #[tokio::test]
    async fn store_get_revoke_round_trip_on_three_database_env() {
        let repo = create_test_repo().await;
        let session = sample_session("usr_1", "hash_roundtrip", 3600);

        repo.store_refresh_token(&session).await.expect("store");
        let loaded = repo
            .get_session_by_refresh_token("hash_roundtrip")
            .await
            .expect("get")
            .expect("session present");
        assert_eq!(loaded.user_id, "usr_1");

        repo.revoke_session("hash_roundtrip").await.expect("revoke");
        let gone = repo
            .get_session_by_refresh_token("hash_roundtrip")
            .await
            .expect("get after revoke");
        assert!(gone.is_none(), "revoked session must be gone");
    }

    #[tokio::test]
    async fn cleanup_sweeps_expired_sessions_and_single_use_records_together() {
        let repo = create_test_repo().await;

        let live = sample_session("usr_1", "hash_cleanup_live", 3600);
        let dead = sample_session("usr_1", "hash_cleanup_dead", -60);
        repo.store_refresh_token(&live).await.expect("store live");
        repo.store_refresh_token(&dead).await.expect("store dead");

        let claimed_live = repo
            .put_single_use(
                "nonce:su_cleanup_live",
                Utc::now() + chrono::Duration::hours(1),
            )
            .await
            .expect("put live record");
        assert!(claimed_live);
        let claimed_dead = repo.put_single_use(
            "nonce:su_cleanup_dead",
            Utc::now() - chrono::Duration::minutes(1),
        );
        // An already-expired claim writes fine; the sweep is what reclaims it.
        claimed_dead.await.expect("put expired record");

        let removed = repo.cleanup_expired_sessions().await.expect("cleanup");
        assert_eq!(
            removed, 2,
            "sweep must count one expired session plus one expired record"
        );

        let live_survives = repo
            .take_single_use("nonce:su_cleanup_live")
            .await
            .expect("take live record after sweep");
        assert!(live_survives, "the live record must survive the sweep");

        let session_survives = repo
            .get_session_by_refresh_token("hash_cleanup_live")
            .await
            .expect("get live session after sweep");
        assert!(
            session_survives.is_some(),
            "cleanup must not touch live sessions"
        );
    }

    #[tokio::test]
    async fn concurrent_store_and_claim_serialize_without_error() {
        let repo = std::sync::Arc::new(create_test_repo().await);

        let mut tasks = tokio::task::JoinSet::new();
        for i in 0..4 {
            let repo = Arc::clone(&repo);
            tasks.spawn(async move {
                let session =
                    sample_session(&format!("usr_race_{i}"), &format!("hash_race_{i}"), 600);
                repo.store_refresh_token(&session)
                    .await
                    .expect("store in race");
                repo.put_single_use(
                    &format!("nonce:race_{i}"),
                    Utc::now() + chrono::Duration::minutes(10),
                )
                .await
                .expect("put in race")
            });
        }

        while let Some(joined) = tasks.join_next().await {
            let won = joined.expect("race task must not panic");
            assert!(
                won,
                "each racing task claims its own distinct key and must win it"
            );
        }
    }

    /// The shared single-use conformance suite, run against LMDB.
    mod conformance_suite {
        use super::*;
        use oidc_exchange_test_utils::single_use_conformance;

        #[tokio::test]
        async fn first_claim_wins_duplicate_loses() {
            let repo = create_test_repo().await;
            single_use_conformance::first_claim_wins_duplicate_loses(&repo).await;
        }

        #[tokio::test]
        async fn consume_live_record_exactly_once() {
            let repo = create_test_repo().await;
            single_use_conformance::consume_live_record_exactly_once(&repo).await;
        }

        #[tokio::test]
        async fn expired_record_is_absent_to_put_and_take() {
            let repo = create_test_repo().await;
            single_use_conformance::expired_record_is_absent_to_put_and_take(&repo).await;
        }

        #[tokio::test]
        async fn concurrent_put_has_exactly_one_winner() {
            let repo = std::sync::Arc::new(create_test_repo().await);
            single_use_conformance::concurrent_put_has_exactly_one_winner(repo).await;
        }

        #[tokio::test]
        async fn concurrent_take_has_exactly_one_winner() {
            let repo = std::sync::Arc::new(create_test_repo().await);
            single_use_conformance::concurrent_take_has_exactly_one_winner(repo).await;
        }

        #[tokio::test]
        async fn cleanup_sweeps_expired_records_and_counts_both_kinds() {
            let repo = create_test_repo().await;
            single_use_conformance::cleanup_sweeps_expired_single_use_records(&repo).await;
        }
    }
}
