use async_trait::async_trait;
use chrono::Utc;
use heed::types::{Bytes, Str};
use heed::{Database, Env, EnvOpenOptions};
use oidc_exchange_core::domain::Session;
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
