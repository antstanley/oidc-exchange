use async_trait::async_trait;
use chrono::Utc;
use heed::types::{Bytes, Str};
use heed::{Database, Env, EnvOpenOptions};
use oidc_exchange_core::domain::{RefreshResolution, Session};
use oidc_exchange_core::error::Error;
use oidc_exchange_core::ports::SessionRepository;
use std::fs;
use tracing::instrument;

/// LMDB-backed session repository using the `heed` crate.
///
/// Two named databases are maintained:
/// - `sessions`: `token_hash -> JSON(Session)`
/// - `user_sessions`: `"{user_id}:{token_hash}" -> ""` (secondary index)
pub struct LmdbSessionRepository {
    env: Env,
    sessions: Database<Str, Bytes>,
    user_sessions: Database<Str, Str>,
}

impl LmdbSessionRepository {
    /// Opens (or creates) an LMDB environment at `path` with the given max size.
    pub fn new(path: &str, max_size_mb: u64) -> Result<Self, Box<dyn std::error::Error>> {
        fs::create_dir_all(path)?;

        let env = unsafe {
            EnvOpenOptions::new()
                .max_dbs(2)
                .map_size((max_size_mb * 1024 * 1024) as usize)
                .open(path)?
        };

        let mut wtxn = env.write_txn()?;
        let sessions: Database<Str, Bytes> = env.create_database(&mut wtxn, Some("sessions"))?;
        let user_sessions: Database<Str, Str> =
            env.create_database(&mut wtxn, Some("user_sessions"))?;
        wtxn.commit()?;

        Ok(Self {
            env,
            sessions,
            user_sessions,
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

    /// Interim classification for task 01: this store holds live generations
    /// only — the `retired_tokens` database and the full
    /// `Superseded`/`Retired`/`Unknown` classification arrive with task 04
    /// (lmdb_session_adapter). Live-or-unknown is therefore the complete,
    /// truthful answer today, and it is strongly consistent: an LMDB read
    /// transaction sees every committed write.
    #[instrument(skip(self), fields(token_hash))]
    async fn resolve_refresh_token(
        &self,
        token_hash: &str,
    ) -> oidc_exchange_core::error::Result<RefreshResolution> {
        assert!(
            !token_hash.is_empty(),
            "resolve_refresh_token: token_hash must not be empty"
        );
        match self.get_session_by_refresh_token(token_hash).await? {
            Some(session) => Ok(RefreshResolution::Live(session)),
            None => Ok(RefreshResolution::Unknown),
        }
    }

    /// Interim rotation for task 01: all reads and writes happen inside one
    /// `heed` write transaction, which is where the compare-and-swap condition
    /// is evaluated. LMDB write transactions are exclusive, so exactly one
    /// caller observes the live generation (SR3) and the delete-plus-insert
    /// commits as a unit or not at all (SR2). Task 04 extends this same
    /// transaction with the retirement-record write that SR4/SR5 need.
    #[instrument(skip(self, replacement), fields(token_hash = %live_hash, user_id = %replacement.user_id))]
    async fn rotate_refresh_token(
        &self,
        live_hash: &str,
        replacement: &Session,
    ) -> oidc_exchange_core::error::Result<bool> {
        assert_ne!(
            live_hash, replacement.refresh_token_hash,
            "rotate_refresh_token: replacement must be a fresh generation"
        );
        assert!(
            !replacement.refresh_token_hash.is_empty(),
            "rotate_refresh_token: replacement hash must not be empty"
        );

        let env = self.env.clone();
        let sessions_db = self.sessions;
        let user_sessions_db = self.user_sessions;
        let live_hash = live_hash.to_owned();
        let replacement = replacement.clone();

        tokio::task::spawn_blocking(move || {
            let json = serde_json::to_vec(&replacement).map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

            let mut wtxn = env.write_txn().map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

            // CAS condition inside the exclusive write transaction: the named
            // hash must still be a live generation.
            let live_user_id = match sessions_db.get(&wtxn, live_hash.as_str()) {
                Ok(Some(bytes)) => {
                    let session: Session =
                        serde_json::from_slice(bytes).map_err(|e| Error::StoreError {
                            detail: e.to_string(),
                        })?;
                    // Precondition: a rotation replaces a generation of the
                    // same family for the same user — anything else would
                    // strand credentials outside their holder's control.
                    assert_eq!(
                        session.family_id, replacement.family_id,
                        "rotate_refresh_token: family mismatch between live and replacement"
                    );
                    assert_eq!(
                        session.user_id, replacement.user_id,
                        "rotate_refresh_token: user mismatch between live and replacement"
                    );
                    Some(session.user_id)
                }
                Ok(None) => None,
                Err(e) => {
                    return Err(Error::StoreError {
                        detail: e.to_string(),
                    })
                }
            };

            let Some(live_user_id) = live_user_id else {
                // The condition failed: a concurrent redemption moved the live
                // generation first. Drop the transaction without writing.
                drop(wtxn);
                return Ok(false);
            };

            sessions_db
                .delete(&mut wtxn, live_hash.as_str())
                .map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })?;
            let live_index_key = LmdbSessionRepository::user_session_key(&live_user_id, &live_hash);
            user_sessions_db
                .delete(&mut wtxn, live_index_key.as_str())
                .map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })?;

            sessions_db
                .put(&mut wtxn, replacement.refresh_token_hash.as_str(), &json)
                .map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })?;
            let replacement_index_key = LmdbSessionRepository::user_session_key(
                &replacement.user_id,
                &replacement.refresh_token_hash,
            );
            user_sessions_db
                .put(&mut wtxn, replacement_index_key.as_str(), "")
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

    /// Interim family revocation for task 01: removes the family's live
    /// generations and returns how many, sweeping their `user_sessions` index
    /// entries in the same write transaction. The retirement records
    /// `revoke_family` must also remove do not exist in this store until task
    /// 04 adds the `retired_tokens` database and `family_index`, so the count
    /// covers live rows only — exactly the removal work this store currently
    /// holds.
    #[instrument(skip(self), fields(family_id))]
    async fn revoke_family(&self, family_id: &str) -> oidc_exchange_core::error::Result<u64> {
        assert!(
            !family_id.is_empty(),
            "revoke_family: family_id must not be empty"
        );

        let env = self.env.clone();
        let sessions_db = self.sessions;
        let user_sessions_db = self.user_sessions;
        let family_id = family_id.to_owned();

        tokio::task::spawn_blocking(move || {
            // Collect the family's live generations from a consistent read,
            // then delete them and their index entries in one write txn so a
            // concurrent rotation cannot resurrect an entry mid-sweep.
            let rtxn = env.read_txn().map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;
            let mut to_delete: Vec<(String, String)> = Vec::new(); // (hash, user_id)
            let iter = sessions_db.iter(&rtxn).map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;
            for result in iter {
                let (key, bytes) = result.map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })?;
                let session: Session =
                    serde_json::from_slice(bytes).map_err(|e| Error::StoreError {
                        detail: e.to_string(),
                    })?;
                if session.family_id == family_id {
                    to_delete.push((key.to_owned(), session.user_id));
                }
            }
            drop(rtxn);

            if to_delete.is_empty() {
                return Ok(0);
            }

            let removed = to_delete.len() as u64;
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
            wtxn.commit().map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

            Ok(removed)
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
            drop(rtxn);

            if to_delete.is_empty() {
                return Ok(0);
            }

            let deleted = to_delete.len() as u64;
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

            wtxn.commit().map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

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
}
