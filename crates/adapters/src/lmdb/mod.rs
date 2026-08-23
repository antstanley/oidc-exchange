use async_trait::async_trait;
use chrono::Utc;
use heed::types::{Bytes, Str};
use heed::{Database, Env, EnvOpenOptions};
use oidc_exchange_core::domain::Session;
use oidc_exchange_core::error::Error;
use oidc_exchange_core::ports::SessionRepository;
use oidc_exchange_core::Secret;
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
    // The refresh-token hash is the session lookup key, so it must never become a span
    // field value: `session` is skipped wholesale and only the permitted `user_id` is
    // recorded. Naming `session` in both `skip(...)` and (implicitly) not in `fields(...)`
    // keeps the redaction independent of argument renames — unlike a bare
    // `fields(token_hash = %session.refresh_token_hash)`, nothing here can re-expose the
    // digest if a parameter is renamed.
    #[instrument(skip(self, session), fields(user_id = %session.user_id))]
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
                .put(&mut wtxn, session.refresh_token_hash.expose(), &json)
                .map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })?;

            let index_key = LmdbSessionRepository::user_session_key(
                &session.user_id,
                session.refresh_token_hash.expose(),
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

    // `token_hash` names the argument in `skip(...)` AND declares an empty schema field of
    // the same name: the field keeps the log schema stable, while the explicit skip (not a
    // reliance on the name collision) guarantees the digest itself can never be captured,
    // even if the parameter is later renamed.
    #[instrument(skip(self, token_hash), fields(token_hash))]
    async fn get_session_by_refresh_token(
        &self,
        token_hash: &Secret<String>,
    ) -> oidc_exchange_core::error::Result<Option<Session>> {
        let env = self.env.clone();
        let sessions_db = self.sessions;
        // The raw digest is needed only here, where the LMDB key is built.
        let token_hash = token_hash.expose().to_owned();

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

    // Same redaction contract as the lookup path: the digest argument is skipped
    // explicitly, and the bare `token_hash` field stays declared-but-empty for schema
    // stability.
    #[instrument(skip(self, token_hash), fields(token_hash))]
    async fn revoke_session(
        &self,
        token_hash: &Secret<String>,
    ) -> oidc_exchange_core::error::Result<()> {
        let env = self.env.clone();
        let sessions_db = self.sessions;
        let user_sessions_db = self.user_sessions;
        // The raw digest is needed only here, where the LMDB keys are built.
        let token_hash = token_hash.expose().to_owned();

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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use std::collections::HashSet;
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;
    use tracing_subscriber::fmt::format::FmtSpan;
    use tracing_subscriber::layer::{Context, Layer};
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::registry::LookupSpan;

    /// Distinctive marker strings planted in a session's sensitive fields; none of them may
    /// ever surface in captured span output. Realistic shape (a hex-looking digest, a
    /// public documentation-range IP) so the assertions exercise real rendering paths.
    const HASH_SENTINEL: &str = "deadbeefcafe0123456789abcdefdeadbeefcafe0123456789abcdef7890";
    const DEVICE_SENTINEL: &str = "sentinel-device-id";
    const USER_AGENT_SENTINEL: &str = "sentinel-user-agent/1.0";
    const IP_SENTINEL: &str = "192.0.2.17";
    const USER_ID_SENTINEL: &str = "usr_span_redaction";

    /// All client-provenance values carried by a session; the write span records neither
    /// them nor the hash.
    const PROVENANCE_SENTINELS: [&str; 3] = [DEVICE_SENTINEL, USER_AGENT_SENTINEL, IP_SENTINEL];

    /// A clonable in-memory writer the fmt subscriber renders into, so tests can assert on
    /// exactly what was produced rather than scraping stdout.
    #[derive(Clone, Default)]
    struct SharedBuffer(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for SharedBuffer {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .expect("capture mutex must not be poisoned")
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// Records the *declared* field schema `(span name, field name)` of every span created
    /// while installed. The fmt formatter renders only fields that actually received a
    /// value — an empty-but-declared schema field like `token_hash` never appears in the
    /// text stream — so schema observability can only be proven against the metadata.
    struct DeclaredFieldsLayer {
        declared: Arc<Mutex<HashSet<(String, String)>>>,
    }

    impl<S> Layer<S> for DeclaredFieldsLayer
    where
        S: tracing::Subscriber + for<'lookup> LookupSpan<'lookup>,
    {
        fn on_new_span(
            &self,
            attrs: &tracing::span::Attributes<'_>,
            _id: &tracing::span::Id,
            _ctx: Context<'_, S>,
        ) {
            let span_name = attrs.metadata().name().to_string();
            let mut declared = self
                .declared
                .lock()
                .expect("capture mutex must not be poisoned");
            for field in attrs.metadata().fields() {
                declared.insert((span_name.clone(), field.name().to_string()));
            }
            assert!(
                !declared.is_empty(),
                "every instrumented span must declare its field schema"
            );
        }
    }

    /// The capture bundle a span-leak test needs: hold `_guard` for the whole test body,
    /// read rendered telemetry from `buffer`, and assert declared schema via `declared`.
    struct SpanCapture {
        _guard: tracing::subscriber::DefaultGuard,
        buffer: SharedBuffer,
        declared: Arc<Mutex<HashSet<(String, String)>>>,
    }

    /// Install a fmt subscriber writing into `buffer` with explicit span-open/close events
    /// enabled, alongside the schema-capture layer. Without `FmtSpan::CLOSE` the stock
    /// subscriber would never render span teardown, letting a broken skip look clean
    /// because nothing was asserted against.
    ///
    /// Keep the returned handle alive for the whole test body: dropping it uninstalls the
    /// thread-local subscriber.
    fn install_capture(buffer: SharedBuffer) -> SpanCapture {
        let declared = Arc::new(Mutex::new(HashSet::new()));
        // The writer closure owns a clone; the test keeps the original for assertions.
        let writer_buffer = buffer.clone();
        let subscriber = tracing_subscriber::registry()
            .with(
                tracing_subscriber::fmt::layer()
                    .with_writer(move || writer_buffer.clone())
                    .with_span_events(FmtSpan::NEW | FmtSpan::CLOSE)
                    .with_ansi(false),
            )
            .with(DeclaredFieldsLayer {
                declared: declared.clone(),
            });
        SpanCapture {
            _guard: tracing::subscriber::set_default(subscriber),
            buffer,
            declared,
        }
    }

    /// Asserts a span declared exactly one of its expected schema fields, so the log-schema
    /// contract survives even though the formatter prints nothing for empty fields.
    fn assert_declares(
        declared: &Mutex<HashSet<(String, String)>>,
        span_name: &str,
        field_name: &str,
    ) {
        let key = (span_name.to_string(), field_name.to_string());
        assert!(
            declared
                .lock()
                .expect("capture mutex must not be poisoned")
                .contains(&key),
            "span {span_name} must keep declaring the {field_name} schema field"
        );
    }

    fn open_repo() -> (LmdbSessionRepository, TempDir) {
        let dir = TempDir::new().expect("temp dir for lmdb environment");
        let path = dir.path().join("data");
        let repo = LmdbSessionRepository::new(path.to_str().expect("utf-8 temp path"), 16)
            .expect("open lmdb environment");
        (repo, dir)
    }

    fn sentinel_session() -> Session {
        let now = Utc::now();
        Session {
            user_id: USER_ID_SENTINEL.to_string(),
            refresh_token_hash: Secret::new(HASH_SENTINEL.to_string()),
            provider: "google".to_string(),
            expires_at: now + Duration::hours(1),
            device_id: Some(DEVICE_SENTINEL.to_string()),
            user_agent: Some(USER_AGENT_SENTINEL.to_string()),
            ip_address: Some(IP_SENTINEL.to_string()),
            created_at: now,
        }
    }

    /// Negative space: neither the hash nor any provenance value may occur anywhere in the
    /// rendered telemetry.
    fn assert_no_sentinels(rendered: &str) {
        assert!(
            !rendered.contains(HASH_SENTINEL),
            "refresh-token hash must never reach span output"
        );
        for provenance in PROVENANCE_SENTINELS {
            assert!(
                !rendered.contains(provenance),
                "client provenance value ({provenance}) must never reach span output"
            );
        }
    }

    fn rendered_output(buffer: &SharedBuffer) -> String {
        let bytes = buffer
            .0
            .lock()
            .expect("capture mutex must not be poisoned")
            .clone();
        String::from_utf8(bytes).expect("captured telemetry is utf-8")
    }

    /// Regression for the LMDB span exposure: across write, lookup, and revoke, the
    /// refresh-token hash (the session lookup key) and the session's client provenance
    /// must never render, while the permitted `user_id` field and the declared-but-empty
    /// `token_hash` schema field stay observable.
    #[tokio::test]
    async fn session_spans_exclude_hash_and_provenance_but_keep_permitted_fields() {
        let buffer = SharedBuffer::default();
        // Single-threaded `#[tokio::test]`: every poll happens on this thread, so the
        // thread-local default subscriber sees every span open and close below.
        let capture = install_capture(buffer);
        let (repo, _dir) = open_repo();
        let session = sentinel_session();

        repo.store_refresh_token(&session)
            .await
            .expect("store_refresh_token");
        let fetched = repo
            .get_session_by_refresh_token(&session.refresh_token_hash)
            .await
            .expect("get_session_by_refresh_token")
            .expect("stored session must be retrievable");
        assert_eq!(fetched.user_id, session.user_id, "lookup must find the row");
        repo.revoke_session(&session.refresh_token_hash)
            .await
            .expect("revoke_session");

        let rendered = rendered_output(&capture.buffer);

        // Non-vacuousness: all three instrumented spans must have both opened and closed
        // inside this capture before any absence claim means anything.
        for span_name in [
            "store_refresh_token",
            "get_session_by_refresh_token",
            "revoke_session",
        ] {
            let mentions = rendered.matches(span_name).count();
            assert!(
                mentions >= 2,
                "span {span_name} must appear at both open and close, found {mentions}"
            );
        }
        assert_eq!(
            rendered.matches("close").count(),
            3,
            "exactly the three driven spans must have closed in this capture"
        );

        // Permitted observability survives: the write span records `user_id`, and the
        // lookup/revoke spans keep their (value-less) `token_hash` log-schema field.
        assert!(
            rendered.contains(&format!("user_id={USER_ID_SENTINEL}")),
            "the write span must still record user_id"
        );
        assert_declares(&capture.declared, "store_refresh_token", "user_id");
        assert_declares(
            &capture.declared,
            "get_session_by_refresh_token",
            "token_hash",
        );
        assert_declares(&capture.declared, "revoke_session", "token_hash");

        assert_no_sentinels(&rendered);
    }

    /// Redaction is a telemetry contract, not data loss: every session field, including
    /// provenance, must still round-trip through the store byte-for-byte.
    #[tokio::test]
    async fn redaction_is_telemetry_only_and_session_data_round_trips() {
        let (repo, _dir) = open_repo();
        let session = sentinel_session();

        repo.store_refresh_token(&session)
            .await
            .expect("store_refresh_token");
        let fetched = repo
            .get_session_by_refresh_token(&session.refresh_token_hash)
            .await
            .expect("get_session_by_refresh_token")
            .expect("stored session must be retrievable");

        assert_eq!(fetched.user_id, session.user_id);
        assert_eq!(fetched.provider, session.provider);
        assert_eq!(fetched.expires_at, session.expires_at);
        assert_eq!(fetched.created_at, session.created_at);
        assert_eq!(fetched.device_id.as_deref(), Some(DEVICE_SENTINEL));
        assert_eq!(fetched.user_agent.as_deref(), Some(USER_AGENT_SENTINEL));
        assert_eq!(fetched.ip_address.as_deref(), Some(IP_SENTINEL));
        // The hash itself round-trips intact — it just must not be *logged*.
        assert!(
            fetched.refresh_token_hash == session.refresh_token_hash,
            "the hash itself round-trips intact — it just must not be logged"
        );

        repo.revoke_session(&session.refresh_token_hash)
            .await
            .expect("revoke_session");
        let gone = repo
            .get_session_by_refresh_token(&session.refresh_token_hash)
            .await
            .expect("post-revoke lookup");
        assert!(gone.is_none());
    }
}
