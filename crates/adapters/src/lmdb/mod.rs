use async_trait::async_trait;
use chrono::{DateTime, Utc};
use heed::types::{Bytes, Str};
use heed::{Database, Env, EnvOpenOptions, RwTxn};
use oidc_exchange_core::domain::{
    is_valid_family_id, RefreshResolution, RetiredRefreshToken, Session,
};
use oidc_exchange_core::error::Error;
use oidc_exchange_core::secret::Secret;
use oidc_exchange_core::ports::SessionRepository;
use std::fs;
use tracing::instrument;

/// Number of keys one write transaction deletes during
/// `cleanup_expired_sessions`. LMDB is copy-on-write, so a delete must
/// allocate dirty pages before it frees old ones: a sweep that deletes
/// everything in one transaction can fail `MDB_MAP_FULL` on a map filled past
/// roughly 95% (the finding this batching answers), while committing in
/// batches keeps freeing pages as it goes. Named, not inlined, because the
/// bound *is* the behaviour.
pub const LMDB_CLEANUP_BATCH_SIZE: usize = 256;

/// How many times the sweeper may halve its transaction width when a batch
/// cannot fit in the map's remaining headroom (`MAP_FULL`). Starting at
/// [`LMDB_CLEANUP_BATCH_SIZE`] and halving reaches single-delete transactions
/// in exactly this many steps, which is the deepest degradation that can make
/// progress; below it the map is out of room entirely and the sweep errors,
/// leaving recovery (raising `max_size_mb`) to the operator.
const CLEANUP_MAX_BATCH_HALVINGS: u32 = 8;

/// `family_index` value marking the hash it is filed under as the family's
/// live generation.
const FAMILY_INDEX_KIND_LIVE: &str = "live";

/// `family_index` value marking the hash it is filed under as a retained
/// retirement record.
const FAMILY_INDEX_KIND_RETIRED: &str = "retired";

/// An expired entry queued for deletion by the batched sweep, carrying
/// everything needed to unhook its index filings.
enum Expired {
    Live(Session),
    Retired(RetiredRefreshToken),
}

/// The four named databases, cloned out for movement into a
/// `spawn_blocking` closure (`heed::Database` is a cheap copyable handle).
#[derive(Clone, Copy)]
struct Dbs {
    sessions: Database<Str, Bytes>,
    user_sessions: Database<Str, Str>,
    retired_tokens: Database<Str, Bytes>,
    family_index: Database<Str, Str>,
}

impl Dbs {
    fn store_err(e: impl std::fmt::Display) -> Error {
        Error::StoreError {
            detail: e.to_string(),
        }
    }
}

/// Delete one live generation and both of its index filings. Returns whether
/// the session row existed. Legacy (sentinel-family) rows carry no
/// family-index entry to remove.
fn remove_live_entry(
    dbs: &Dbs,
    wtxn: &mut RwTxn,
    token_hash: &str,
    session: &Session,
) -> oidc_exchange_core::error::Result<bool> {
    remove_live_entry_raw(dbs, wtxn, token_hash, session).map_err(Dbs::store_err)
}

/// Delete one retirement record and its family-index filing. Returns whether
/// the record existed.
fn remove_retired_entry(
    dbs: &Dbs,
    wtxn: &mut RwTxn,
    record: &RetiredRefreshToken,
) -> oidc_exchange_core::error::Result<bool> {
    remove_retired_entry_raw(dbs, wtxn, record).map_err(Dbs::store_err)
}

/// [`remove_live_entry`] with the raw error type, for [`commit_deletion_batch`]
/// whose caller must distinguish `MAP_FULL` from every other failure.
fn remove_live_entry_raw(
    dbs: &Dbs,
    wtxn: &mut RwTxn,
    token_hash: &str,
    session: &Session,
) -> Result<bool, heed::Error> {
    let existed = dbs.sessions.delete(wtxn, token_hash)?;
    let index_key = user_session_key(&session.user_id, token_hash);
    dbs.user_sessions.delete(wtxn, &index_key)?;
    if !session.family_id.is_empty() {
        let family_key = family_index_key(&session.family_id, token_hash);
        dbs.family_index.delete(wtxn, &family_key)?;
    }
    Ok(existed)
}

/// [`remove_retired_entry`] with the raw error type, for
/// [`commit_deletion_batch`] whose caller must distinguish `MAP_FULL` from
/// every other failure.
fn remove_retired_entry_raw(
    dbs: &Dbs,
    wtxn: &mut RwTxn,
    record: &RetiredRefreshToken,
) -> Result<bool, heed::Error> {
    let existed = dbs
        .retired_tokens
        .delete(wtxn, &record.refresh_token_hash)?;
    let family_key = family_index_key(&record.family_id, &record.refresh_token_hash);
    dbs.family_index.delete(wtxn, &family_key)?;
    Ok(existed)
}

/// Delete one slice of the sweep queue inside a single committed write
/// transaction, returning the number of entries actually removed. The raw
/// `heed::Error` is returned so the caller can distinguish a `MAP_FULL`
/// condition (shrink and retry) from every other failure (surface it).
fn commit_deletion_batch(env: &Env, dbs: &Dbs, batch: &[Expired]) -> Result<u64, heed::Error> {
    let mut wtxn = env.write_txn()?;
    let mut removed: u64 = 0;
    for entry in batch {
        let existed = match entry {
            Expired::Live(session) => {
                remove_live_entry_raw(dbs, &mut wtxn, session.refresh_token_hash.expose(), session)?
            }
            Expired::Retired(record) => remove_retired_entry_raw(dbs, &mut wtxn, record)?,
        };
        removed += u64::from(existed);
    }
    wtxn.commit()?;
    Ok(removed)
}

/// Build the retirement record a winning rotation writes for `live`. The
/// successor inherits the family identity; the deadline is
/// `min(now + reuse_retention, family expires_at)` so no record outlives its
/// family. Legacy rows must never reach this helper — there is no prior
/// generation for them to be detected against.
fn retirement_record(
    live_hash: &str,
    live: &Session,
    replacement: &Session,
    reuse_retention_secs: u64,
    now: DateTime<Utc>,
) -> RetiredRefreshToken {
    assert!(
        live.refresh_token_hash.expose() == live_hash,
        "retirement record must name the presented hash"
    );
    assert!(
        is_valid_family_id(&live.family_id),
        "a legacy row must not produce a retirement record: {:?}",
        live.family_id
    );
    RetiredRefreshToken {
        refresh_token_hash: live_hash.to_string(),
        family_id: live.family_id.clone(),
        user_id: live.user_id.clone(),
        successor_hash: replacement.refresh_token_hash.expose().clone(),
        retired_at: now,
        expires_at: RetiredRefreshToken::retention_deadline(
            now,
            reuse_retention_secs,
            replacement.expires_at,
        ),
    }
}

/// LMDB-backed session repository using the `heed` crate.
///
/// Five named databases are maintained:
/// - `sessions`: `token_hash -> JSON(Session)` (the live generations)
/// - `user_sessions`: `"{user_id}:{token_hash}" -> ""` (revoke-all index)
/// - `retired_tokens`: `token_hash -> JSON(RetiredRefreshToken)` (reuse
///   detection)
/// - `family_index`: `"{family_id}\0{token_hash}" -> "live"|"retired"`
///   (`revoke_family`'s enumeration; the NUL separator cannot appear in a
///   family id or hash, so no key is a prefix of another family's keys)
/// - `single_use`: `digest_key -> RFC3339(expires_at)` for single-use records
///   (nonces, assertion-replay markers)
///
/// Every mutation touches all the databases its effect spans inside one write
/// transaction, which is what makes rotation atomic (SR2) and revocation
/// complete (SR5).
pub struct LmdbSessionRepository {
    env: Env,
    sessions: Database<Str, Bytes>,
    user_sessions: Database<Str, Str>,
    retired_tokens: Database<Str, Bytes>,
    family_index: Database<Str, Str>,
    /// How long a retirement record stays readable after its rotation:
    /// `retired_at + reuse_retention_secs`, capped per record at the family's
    /// absolute `expires_at` by [`RetiredRefreshToken::retention_deadline`].
    /// Resolved from `[token] refresh_reuse_retention` at bootstrap; injected
    /// here because the store, not the caller, stamps every record's deadline.
    reuse_retention_secs: u64,
    /// Single-use records (nonces, assertion-replay markers), keyed by
    /// namespaced digest. A separate named database keeps their key space
    /// disjoint from session token hashes.
    single_use: Database<Str, Str>,
}

/// Build the composite key used in the `user_sessions` index.
fn user_session_key(user_id: &str, token_hash: &str) -> String {
    format!("{user_id}:{token_hash}")
}

/// Build the composite key used in the `family_index` database.
fn family_index_key(family_id: &str, token_hash: &str) -> String {
    format!("{family_id}\0{token_hash}")
}

impl LmdbSessionRepository {
    /// Opens (or creates) an LMDB environment at `path` with the given max
    /// size and reuse-retention window.
    pub fn new(
        path: &str,
        max_size_mb: u64,
        reuse_retention_secs: u64,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        assert!(
            reuse_retention_secs > 0,
            "reuse retention must be greater than zero"
        );
        fs::create_dir_all(path)?;

        let env = unsafe {
            EnvOpenOptions::new()
                .max_dbs(5)
                .map_size((max_size_mb * 1024 * 1024) as usize)
                .open(path)?
        };

        let mut wtxn = env.write_txn()?;
        let sessions: Database<Str, Bytes> = env.create_database(&mut wtxn, Some("sessions"))?;
        let user_sessions: Database<Str, Str> =
            env.create_database(&mut wtxn, Some("user_sessions"))?;
        let retired_tokens: Database<Str, Bytes> =
            env.create_database(&mut wtxn, Some("retired_tokens"))?;
        let family_index: Database<Str, Str> =
            env.create_database(&mut wtxn, Some("family_index"))?;
        let single_use: Database<Str, Str> = env.create_database(&mut wtxn, Some("single_use"))?;
        wtxn.commit()?;

        Ok(Self {
            env,
            sessions,
            user_sessions,
            retired_tokens,
            family_index,
            reuse_retention_secs,
            single_use,
        })
    }

    fn databases(&self) -> Dbs {
        Dbs {
            sessions: self.sessions,
            user_sessions: self.user_sessions,
            retired_tokens: self.retired_tokens,
            family_index: self.family_index,
        }
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
        // Precondition: callers mint well-formed family ids; the empty-string
        // sentinel is the one non-well-formed value accepted (a pre-rotation
        // legacy row, which belongs to no family).
        assert!(
            session.family_id.is_empty() || is_valid_family_id(&session.family_id),
            "store_refresh_token: malformed family id {:?}",
            session.family_id
        );
        assert!(
            !session.refresh_token_hash.expose().is_empty(),
            "store_refresh_token: refresh_token_hash must not be empty"
        );

        let env = self.env.clone();
        let dbs = self.databases();
        let session = session.clone();

        tokio::task::spawn_blocking(move || {
            let json = serde_json::to_vec(&session).map_err(Dbs::store_err)?;

            let mut wtxn = env.write_txn().map_err(Dbs::store_err)?;

            dbs.sessions
                .put(&mut wtxn, session.refresh_token_hash.expose(), &json)
                .map_err(Dbs::store_err)?;

            let index_key = user_session_key(&session.user_id, session.refresh_token_hash.expose());
            dbs.user_sessions
                .put(&mut wtxn, &index_key, "")
                .map_err(Dbs::store_err)?;

            // A sentinel-family (legacy) row gets no family-index entry: it
            // belongs to no family, and `revoke_family` rejects the empty id,
            // so an entry filed under "" could never be addressed.
            if !session.family_id.is_empty() {
                let family_key = family_index_key(&session.family_id, session.refresh_token_hash.expose());
                dbs.family_index
                    .put(&mut wtxn, &family_key, FAMILY_INDEX_KIND_LIVE)
                    .map_err(Dbs::store_err)?;
            }

            wtxn.commit().map_err(Dbs::store_err)?;

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
            let rtxn = env.read_txn().map_err(Dbs::store_err)?;

            let maybe_bytes = sessions_db
                .get(&rtxn, &token_hash)
                .map_err(Dbs::store_err)?;

            match maybe_bytes {
                Some(bytes) => {
                    let session: Session = serde_json::from_slice(bytes).map_err(Dbs::store_err)?;
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

    /// Classify `token_hash` against live generations first, then retained
    /// retirement records (SR1). An LMDB read transaction observes every
    /// committed write, so the answer is strongly consistent. A record past
    /// its retention deadline answers `Unknown` until the sweep physically
    /// deletes it — reuse detection must not fire on a window that has
    /// closed.
    #[instrument(skip(self, token_hash), fields(token_hash))]
    async fn resolve_refresh_token(
        &self,
        token_hash: &str,
    ) -> oidc_exchange_core::error::Result<RefreshResolution> {
        assert!(
            !token_hash.is_empty(),
            "resolve_refresh_token: token_hash must not be empty"
        );

        let env = self.env.clone();
        let dbs = self.databases();
        let token_hash = token_hash.to_owned();

        tokio::task::spawn_blocking(move || {
            let rtxn = env.read_txn().map_err(Dbs::store_err)?;

            if let Some(bytes) = dbs
                .sessions
                .get(&rtxn, &token_hash)
                .map_err(Dbs::store_err)?
            {
                let session: Session = serde_json::from_slice(bytes).map_err(Dbs::store_err)?;
                return Ok(RefreshResolution::Live(session));
            }

            let Some(bytes) = dbs
                .retired_tokens
                .get(&rtxn, &token_hash)
                .map_err(Dbs::store_err)?
            else {
                return Ok(RefreshResolution::Unknown);
            };
            let record: RetiredRefreshToken =
                serde_json::from_slice(bytes).map_err(Dbs::store_err)?;
            if record.expires_at <= Utc::now() {
                return Ok(RefreshResolution::Unknown);
            }

            match dbs
                .sessions
                .get(&rtxn, &record.successor_hash)
                .map_err(Dbs::store_err)?
            {
                Some(successor_bytes) => {
                    let successor: Session =
                        serde_json::from_slice(successor_bytes).map_err(Dbs::store_err)?;
                    // Pairing invariant of `rotate_refresh_token`: a successor
                    // pointer always names a generation of the same family.
                    assert_eq!(
                        successor.family_id, record.family_id,
                        "store corruption: successor of {} names family {} but lives in {}",
                        record.refresh_token_hash, record.family_id, successor.family_id
                    );
                    Ok(RefreshResolution::Superseded {
                        live: successor,
                        retired_at: record.retired_at,
                    })
                }
                None => Ok(RefreshResolution::Retired {
                    family_id: record.family_id,
                    user_id: record.user_id,
                    retired_at: record.retired_at,
                }),
            }
        })
        .await
        .map_err(|e| Error::StoreError {
            detail: e.to_string(),
        })?
    }

    /// All reads and writes happen inside one `heed` write transaction, which
    /// is where the compare-and-swap condition is evaluated (SR2/SR3/SR4).
    /// LMDB write transactions are exclusive, so exactly one caller observes
    /// the live generation, and the delete, retirement write, replacement
    /// install, and all three index updates commit as a unit or not at all.
    ///
    /// A live row carrying the empty-family sentinel is a pre-rotation legacy
    /// row: its first redemption deletes it and installs the replacement
    /// *without* a retirement record — there is no prior generation to detect
    /// reuse against — and the replacement carries whatever family the caller
    /// minted. The store never invents one.
    #[instrument(skip(self, live_hash, replacement), fields(token_hash, user_id = %replacement.user_id))]
    async fn rotate_refresh_token(
        &self,
        live_hash: &str,
        replacement: &Session,
    ) -> oidc_exchange_core::error::Result<bool> {
        assert!(
            is_valid_family_id(&replacement.family_id),
            "rotate_refresh_token: malformed replacement family id {:?}",
            replacement.family_id
        );
        assert!(
            live_hash != replacement.refresh_token_hash.expose().as_str(),
            "rotate_refresh_token: replacement must be a fresh generation"
        );

        let env = self.env.clone();
        let dbs = self.databases();
        let reuse_retention_secs = self.reuse_retention_secs;
        let live_hash = live_hash.to_owned();
        let replacement = replacement.clone();

        tokio::task::spawn_blocking(move || {
            let mut wtxn = env.write_txn().map_err(Dbs::store_err)?;

            // CAS condition inside the exclusive write transaction: the named
            // hash must still be a live generation.
            let live: Session = match dbs
                .sessions
                .get(&wtxn, live_hash.as_str())
                .map_err(Dbs::store_err)?
            {
                Some(bytes) => serde_json::from_slice(bytes).map_err(Dbs::store_err)?,
                None => {
                    // The condition failed: a concurrent redemption moved the
                    // live generation first. Drop the transaction without
                    // writing.
                    drop(wtxn);
                    return Ok(false);
                }
            };
            let legacy_row = live.family_id.is_empty();
            if !legacy_row {
                // A rotation replaces a generation of the same family —
                // anything else would strand credentials outside their
                // holder's control.
                assert_eq!(
                    live.family_id, replacement.family_id,
                    "rotate_refresh_token: family mismatch between live and replacement"
                );
            }
            assert_eq!(
                live.user_id, replacement.user_id,
                "rotate_refresh_token: user mismatch between live and replacement"
            );
            // The replacement must be a fresh generation: colliding with any
            // existing live row or retirement record is a caller bug, not a
            // state to overwrite silently.
            assert!(
                dbs.sessions
                    .get(&wtxn, replacement.refresh_token_hash.expose().as_str())
                    .map_err(Dbs::store_err)?
                    .is_none(),
                "rotate_refresh_token: replacement hash already exists as a live session"
            );
            assert!(
                dbs.retired_tokens
                    .get(&wtxn, replacement.refresh_token_hash.expose().as_str())
                    .map_err(Dbs::store_err)?
                    .is_none(),
                "rotate_refresh_token: replacement hash already exists as a retired record"
            );

            remove_live_entry(&dbs, &mut wtxn, &live_hash, &live)?;

            if !legacy_row {
                let record = retirement_record(
                    &live_hash,
                    &live,
                    &replacement,
                    reuse_retention_secs,
                    Utc::now(),
                );
                let record_json = serde_json::to_vec(&record).map_err(Dbs::store_err)?;
                dbs.retired_tokens
                    .put(&mut wtxn, record.refresh_token_hash.as_str(), &record_json)
                    .map_err(Dbs::store_err)?;
                let retired_family_key =
                    family_index_key(&record.family_id, &record.refresh_token_hash);
                dbs.family_index
                    .put(&mut wtxn, &retired_family_key, FAMILY_INDEX_KIND_RETIRED)
                    .map_err(Dbs::store_err)?;
            }

            install_live_entry(&dbs, &mut wtxn, &replacement)?;

            wtxn.commit().map_err(Dbs::store_err)?;

            Ok(true)
        })
        .await
        .map_err(|e| Error::StoreError {
            detail: e.to_string(),
        })?
    }

    #[instrument(skip(self, token_hash), fields(token_hash))]
    async fn revoke_session(&self, token_hash: &Secret<String>) -> oidc_exchange_core::error::Result<()> {
        let token_hash = token_hash.expose().as_str();
        assert!(
            !token_hash.is_empty(),
            "revoke_session: token_hash must not be empty"
        );

        let env = self.env.clone();
        let dbs = self.databases();
        let token_hash = token_hash.to_owned();

        tokio::task::spawn_blocking(move || {
            let mut wtxn = env.write_txn().map_err(Dbs::store_err)?;

            // Read inside the write transaction so the entry cannot move
            // between the lookup and the delete (write transactions are
            // exclusive, so nothing can interleave anyway).
            let session: Option<Session> = match dbs
                .sessions
                .get(&wtxn, token_hash.as_str())
                .map_err(Dbs::store_err)?
            {
                Some(bytes) => Some(serde_json::from_slice(bytes).map_err(Dbs::store_err)?),
                None => None,
            };

            if let Some(session) = session {
                remove_live_entry(&dbs, &mut wtxn, &token_hash, &session)?;
                wtxn.commit().map_err(Dbs::store_err)?;
            }
            // An unknown or already-removed hash is idempotently a no-op;
            // retirement records are not touched (port contract).

            Ok(())
        })
        .await
        .map_err(|e| Error::StoreError {
            detail: e.to_string(),
        })?
    }

    /// Remove the family's live generation and every retained retirement
    /// record, returning the combined count (SR5), enumerating through the
    /// `family_index` inside one write transaction so a concurrent rotation
    /// cannot resurrect an entry mid-sweep. Idempotent: an unknown (but
    /// well-formed) family id removes nothing and returns `Ok(0)`.
    #[instrument(skip(self), fields(family_id))]
    async fn revoke_family(&self, family_id: &str) -> oidc_exchange_core::error::Result<u64> {
        assert!(
            is_valid_family_id(family_id),
            "revoke_family: malformed family id {family_id:?}"
        );

        let env = self.env.clone();
        let dbs = self.databases();
        let family_id = family_id.to_owned();

        tokio::task::spawn_blocking(move || {
            let mut wtxn = env.write_txn().map_err(Dbs::store_err)?;
            let removed = revoke_family_wtxn(&dbs, &mut wtxn, &family_id)?;
            wtxn.commit().map_err(Dbs::store_err)?;
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
            let rtxn = env.read_txn().map_err(Dbs::store_err)?;

            let mut count: u64 = 0;
            let iter = sessions_db.iter(&rtxn).map_err(Dbs::store_err)?;

            for result in iter {
                let (_key, bytes) = result.map_err(Dbs::store_err)?;
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

    /// Delete every expired session and expired retirement record, committing
    /// the deletes in fixed-size [`LMDB_CLEANUP_BATCH_SIZE`] batches rather
    /// than one all-or-nothing transaction: each commit frees pages back to
    /// the map, so the sweep stays effective on a map filled near capacity
    /// where a single transaction would fail `MDB_MAP_FULL`. The count is the
    /// number of entries actually removed (an entry concurrently revoked
    /// between the read and its batch is simply no longer there).
    #[instrument(skip(self))]
    async fn cleanup_expired_sessions(&self) -> oidc_exchange_core::error::Result<u64> {
        let env = self.env.clone();
        let dbs = self.databases();
        let single_use_db = self.single_use;

        tokio::task::spawn_blocking(move || {
            let now = Utc::now();

            // Read phase: collect the expired entries carrying everything the
            // delete phase needs to unhook their index filings.
            let mut expired: Vec<Expired> = Vec::new();
            {
                let rtxn = env.read_txn().map_err(Dbs::store_err)?;
                let iter = dbs.sessions.iter(&rtxn).map_err(Dbs::store_err)?;
                for result in iter {
                    let (_key, bytes) = result.map_err(Dbs::store_err)?;
                    if let Ok(session) = serde_json::from_slice::<Session>(bytes) {
                        if session.expires_at <= now {
                            expired.push(Expired::Live(session));
                        }
                    }
                }
                let iter = dbs.retired_tokens.iter(&rtxn).map_err(Dbs::store_err)?;
                for result in iter {
                    let (_key, bytes) = result.map_err(Dbs::store_err)?;
                    if let Ok(record) = serde_json::from_slice::<RetiredRefreshToken>(bytes) {
                        if record.expires_at <= now {
                            expired.push(Expired::Retired(record));
                        }
                    }
                }
            }

            // Delete phase: one committed write transaction per batch, each
            // commit freeing its pages before the next begins. A batch that
            // cannot fit in the map's remaining headroom (`MDB_MAP_FULL`) is
            // retried at half width — down to single-delete transactions,
            // bounded by [`CLEANUP_MAX_BATCH_HALVINGS`] — so the sweep
            // degrades gracefully instead of wedging on the very maps it
            // exists to rescue.
            let mut deleted: u64 = 0;
            let mut cursor: usize = 0;
            while cursor < expired.len() {
                let mut width = LMDB_CLEANUP_BATCH_SIZE.min(expired.len() - cursor);
                let mut halvings: u32 = 0;
                loop {
                    match commit_deletion_batch(&env, &dbs, &expired[cursor..cursor + width]) {
                        Ok(removed) => {
                            deleted += removed;
                            break;
                        }
                        Err(err) => {
                            let is_map_full =
                                matches!(err, heed::Error::Mdb(heed::MdbError::MapFull));
                            if is_map_full && width > 1 && halvings < CLEANUP_MAX_BATCH_HALVINGS {
                                // The map has no room for this batch's
                                // copy-on-write pages; halve and retry. The
                                // failed transaction wrote nothing.
                                halvings += 1;
                                width = std::cmp::max(width / 2, 1);
                                continue;
                            }
                            return Err(Dbs::store_err(err));
                        }
                    }
                }
                cursor += width;
            }

            // Sweep expired single-use records too: LMDB has no native
            // expiry, so this sweep is their only space reclamation. The
            // claim operations already treat an expired record as absent, so
            // a skipped sweep never affects correctness.
            let mut single_use_to_delete: Vec<String> = Vec::new();
            {
                let rtxn = env.read_txn().map_err(Dbs::store_err)?;
                let iter = single_use_db.iter(&rtxn).map_err(Dbs::store_err)?;
                for result in iter {
                    let (key, expires_at_str) = result.map_err(Dbs::store_err)?;
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
            }
            if !single_use_to_delete.is_empty() {
                let mut wtxn = env.write_txn().map_err(Dbs::store_err)?;
                for key in &single_use_to_delete {
                    single_use_db
                        .delete(&mut wtxn, key.as_str())
                        .map_err(Dbs::store_err)?;
                }
                wtxn.commit().map_err(Dbs::store_err)?;
                deleted += single_use_to_delete.len() as u64;
            }

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
        assert!(
            !user_id.is_empty(),
            "revoke_all_user_sessions: user_id must not be empty"
        );

        let env = self.env.clone();
        let dbs = self.databases();
        let user_id = user_id.to_owned();

        tokio::task::spawn_blocking(move || {
            let mut wtxn = env.write_txn().map_err(Dbs::store_err)?;
            let prefix = format!("{user_id}:");

            // The user's live generations come off the user_sessions index;
            // their retained retirement records are swept from the
            // `retired_tokens` database by owner (a full scan, bounded by the
            // retention window's steady-state size).
            let mut live: Vec<(String, Session)> = Vec::new();
            {
                let iter = dbs
                    .user_sessions
                    .prefix_iter(&wtxn, &prefix)
                    .map_err(Dbs::store_err)?;
                for result in iter {
                    let (key, _val) = result.map_err(Dbs::store_err)?;
                    let token_hash = key
                        .strip_prefix(&prefix)
                        .expect("prefix_iter keys carry the prefix")
                        .to_owned();
                    let bytes = dbs
                        .sessions
                        .get(&wtxn, token_hash.as_str())
                        .map_err(Dbs::store_err)?
                        .ok_or_else(|| {
                            Dbs::store_err(format!(
                                "user_sessions names live session {token_hash} but none is stored"
                            ))
                        })?;
                    let session: Session = serde_json::from_slice(bytes).map_err(Dbs::store_err)?;
                    assert_eq!(
                        session.user_id, user_id,
                        "user_sessions entry must agree with the stored session's owner"
                    );
                    live.push((token_hash, session));
                }
            }
            let mut retired: Vec<RetiredRefreshToken> = Vec::new();
            {
                let iter = dbs.retired_tokens.iter(&wtxn).map_err(Dbs::store_err)?;
                for result in iter {
                    let (_key, bytes) = result.map_err(Dbs::store_err)?;
                    let record: RetiredRefreshToken =
                        serde_json::from_slice(bytes).map_err(Dbs::store_err)?;
                    if record.user_id == user_id {
                        retired.push(record);
                    }
                }
            }

            for (token_hash, session) in &live {
                remove_live_entry(&dbs, &mut wtxn, token_hash, session)?;
            }
            for record in &retired {
                remove_retired_entry(&dbs, &mut wtxn, record)?;
            }

            wtxn.commit().map_err(Dbs::store_err)?;

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
        LmdbSessionRepository::new(&path_str, 16, 3600).expect("open lmdb env")
    }

    fn sample_session(user_id: &str, hash: &str, ttl_seconds: i64) -> Session {
        let now = Utc::now();
        Session {
            user_id: user_id.to_string(),
            family_id: oidc_exchange_core::domain::new_family_id(),
            generation: 0,
            rotated_at: None,
            refresh_token_hash: Secret::new(hash.to_string()),
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
            .get_session_by_refresh_token(&Secret::new("hash_roundtrip".to_string()))
            .await
            .expect("get")
            .expect("session present");
        assert_eq!(loaded.user_id, "usr_1");

        repo.revoke_session(&Secret::new("hash_roundtrip".to_string())).await.expect("revoke");
        let gone = repo
            .get_session_by_refresh_token(&Secret::new("hash_roundtrip".to_string()))
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
            .get_session_by_refresh_token(&Secret::new("hash_cleanup_live".to_string()))
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

/// Write one live generation and both of its index filings into an open write
/// transaction. A sentinel-family (legacy) row is stored without a
/// family-index entry, mirroring `store_refresh_token`.
fn install_live_entry(
    dbs: &Dbs,
    wtxn: &mut RwTxn,
    session: &Session,
) -> oidc_exchange_core::error::Result<()> {
    let json = serde_json::to_vec(session).map_err(Dbs::store_err)?;
    dbs.sessions
        .put(wtxn, session.refresh_token_hash.expose(), &json)
        .map_err(Dbs::store_err)?;
    let index_key = user_session_key(&session.user_id, session.refresh_token_hash.expose());
    dbs.user_sessions
        .put(wtxn, &index_key, "")
        .map_err(Dbs::store_err)?;
    if !session.family_id.is_empty() {
        let family_key = family_index_key(&session.family_id, session.refresh_token_hash.expose());
        dbs.family_index
            .put(wtxn, &family_key, FAMILY_INDEX_KIND_LIVE)
            .map_err(Dbs::store_err)?;
    }
    Ok(())
}

/// Enumerate and remove one family's entries through the family index inside
/// an open write transaction. Split from [`SessionRepository::revoke_family`]
/// so the port method stays within the line budget and tests can exercise the
/// transaction body directly.
fn revoke_family_wtxn(
    dbs: &Dbs,
    wtxn: &mut RwTxn,
    family_id: &str,
) -> oidc_exchange_core::error::Result<u64> {
    // Collect the family's entries from this same write transaction (the
    // iterator's immutable borrow ends before the deletes), then remove each
    // one and its index filing.
    let prefix = format!("{family_id}\0");
    let mut live_hashes: Vec<String> = Vec::new();
    let mut retired_hashes: Vec<String> = Vec::new();
    {
        let iter = dbs
            .family_index
            .prefix_iter(wtxn, &prefix)
            .map_err(Dbs::store_err)?;
        for result in iter {
            let (key, kind) = result.map_err(Dbs::store_err)?;
            let token_hash = key
                .strip_prefix(&prefix)
                .expect("prefix_iter keys carry the prefix")
                .to_owned();
            if kind == FAMILY_INDEX_KIND_LIVE {
                live_hashes.push(token_hash);
            } else {
                assert_eq!(
                    kind, FAMILY_INDEX_KIND_RETIRED,
                    "unknown family_index kind {kind:?}"
                );
                retired_hashes.push(token_hash);
            }
        }
    }

    let mut removed: u64 = 0;
    for token_hash in &live_hashes {
        let bytes = dbs
            .sessions
            .get(wtxn, token_hash.as_str())
            .map_err(Dbs::store_err)?
            .ok_or_else(|| {
                Dbs::store_err(format!(
                    "family_index names live session {token_hash} but none is stored"
                ))
            })?;
        let session: Session = serde_json::from_slice(bytes).map_err(Dbs::store_err)?;
        assert_eq!(
            session.family_id, family_id,
            "family_index entry must agree with the stored session's family"
        );
        remove_live_entry(dbs, wtxn, token_hash, &session)?;
        removed += 1;
    }
    for token_hash in &retired_hashes {
        let bytes = dbs
            .retired_tokens
            .get(wtxn, token_hash.as_str())
            .map_err(Dbs::store_err)?
            .ok_or_else(|| {
                Dbs::store_err(format!(
                    "family_index names retired record {token_hash} but none is stored"
                ))
            })?;
        let record: RetiredRefreshToken = serde_json::from_slice(bytes).map_err(Dbs::store_err)?;
        assert_eq!(
            record.family_id, family_id,
            "family_index entry must agree with the stored record's family"
        );
        remove_retired_entry(dbs, wtxn, &record)?;
        removed += 1;
    }

    Ok(removed)
}

#[cfg(test)]
mod single_use_tests {
    use super::*;
    use oidc_exchange_test_utils::session_contract::{self, family_chain, fixture_family_id};
    use tempfile::TempDir;

    /// Reuse-retention window used by every test repository: one hour — short
    /// enough that deadline arithmetic stays inside a test's lifetime, and
    /// positive per the constructor's precondition.
    const TEST_REUSE_RETENTION_SECS: u64 = 3600;

    /// Generous bound on the fill loop in
    /// [`cleanup_reclaims_a_map_filled_near_capacity_in_batches`]: far more
    /// attempts than the smallest map could ever take, so the loop exits on
    /// its occupancy target, never on the bound.
    const FILL_MAX_ATTEMPTS: usize = 100_000;

    /// Fraction (of the map size) at which the fill loop stops seeding. Past
    /// roughly this occupancy, copy-on-write pressure makes wide delete
    /// transactions start failing `MDB_MAP_FULL` (the finding reports the
    /// shipped single-transaction sweep wedging at ≥99%), which is the
    /// regime the batched sweep must survive. If an insert already fails
    /// before the target is reached, the loop stops there instead: a map
    /// wedged past that point admits no transaction at all, and recovery is
    /// raising `max_size_mb` (documented, and out of the sweeper's reach).
    const FILL_TARGET_FRACTION: f64 = 0.90;

    /// Lowest occupancy the fill loop accepts as "near capacity". Below it,
    /// the regression would say nothing about copy-on-write pressure.
    const FILL_FLOOR_FRACTION: f64 = 0.85;

    struct TestRepo {
        repo: LmdbSessionRepository,
        // LMDB memory-maps the directory; the guard keeps it alive for the
        // whole test so the files never disappear underneath the env.
        _dir: TempDir,
    }

    impl std::ops::Deref for TestRepo {
        type Target = LmdbSessionRepository;

        fn deref(&self) -> &Self::Target {
            &self.repo
        }
    }

    async fn create_test_repo() -> TestRepo {
        create_test_repo_with_map_mb(64).await
    }

    async fn create_test_repo_with_map_mb(max_size_mb: u64) -> TestRepo {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("sessions.lmdb");
        let repo = LmdbSessionRepository::new(
            path.to_str().expect("utf8 path"),
            max_size_mb,
            TEST_REUSE_RETENTION_SECS,
        )
        .expect("open LMDB environment");
        TestRepo { repo, _dir: dir }
    }

    /// Store one distinct, already-expired session, returning the store
    /// outcome so the fill loop can detect the insertion boundary.
    async fn seed_expired_session(
        repo: &LmdbSessionRepository,
        index: usize,
    ) -> oidc_exchange_core::error::Result<()> {
        let base = Utc::now() - chrono::Duration::hours(2);
        let session = session_contract::generation_session(
            "usr_fill",
            &fixture_family_id(&format!("lmdb-fill:{index}")),
            0,
            format!("fill-hash-{index:016x}"),
            base,
            base,
            None,
        );
        repo.store_refresh_token(&session).await
    }

    /// Fraction of the map currently occupied, from LMDB's own accounting:
    /// last allocated page number times the page size over the map size.
    fn map_occupancy(repo: &LmdbSessionRepository) -> f64 {
        let info = repo.env.info();
        let stat = repo.env.stat();
        let used_bytes = (info.last_page_number as u64 + 1) * u64::from(stat.page_size);
        used_bytes as f64 / info.map_size as f64
    }

    /// Rewrite every stored timestamp backwards so every session and every
    /// retirement record becomes expired in place — the moral equivalent of
    /// the SQL tests' `UPDATE … SET expires_at`, done through the databases
    /// directly because the port has no expiry-editing operation.
    fn expire_everything_in_place(repo: &LmdbSessionRepository) {
        let dbs = repo.databases();
        let past = Utc::now() - chrono::Duration::hours(2);
        let mut wtxn = repo.env.write_txn().expect("write txn");

        let mut sessions: Vec<(String, Session)> = Vec::new();
        {
            let iter = dbs.sessions.iter(&wtxn).expect("iterate sessions");
            for result in iter {
                let (key, bytes) = result.expect("session entry");
                let mut session: Session = serde_json::from_slice(bytes).expect("parse session");
                session.expires_at = past;
                sessions.push((key.to_owned(), session));
            }
        }
        for (key, session) in &sessions {
            let json = serde_json::to_vec(session).expect("serialize session");
            dbs.sessions
                .put(&mut wtxn, key, &json)
                .expect("rewrite session");
        }

        let mut records: Vec<(String, RetiredRefreshToken)> = Vec::new();
        {
            let iter = dbs.retired_tokens.iter(&wtxn).expect("iterate retired");
            for result in iter {
                let (key, bytes) = result.expect("retired entry");
                let mut record: RetiredRefreshToken =
                    serde_json::from_slice(bytes).expect("parse record");
                record.expires_at = past;
                records.push((key.to_owned(), record));
            }
        }
        for (key, record) in &records {
            let json = serde_json::to_vec(record).expect("serialize record");
            dbs.retired_tokens
                .put(&mut wtxn, key, &json)
                .expect("rewrite record");
        }

        wtxn.commit().expect("commit expiry rewrite");
    }

    #[tokio::test]
    async fn lmdb_session_store_meets_sr1_through_sr5() {
        let test = create_test_repo().await;
        session_contract::assert_full_conformance(&test.repo, "lmdb-session-conformance").await;
    }

    /// A legacy row's first redemption swaps atomically but writes no
    /// retirement record — there is no prior generation to detect reuse
    /// against — and the presented hash reads Unknown afterwards. The
    /// honest-count probe: revoking the replacement's family afterwards
    /// removes exactly one entry (the replacement itself); a stray retirement
    /// record would make it two.
    #[tokio::test]
    async fn legacy_row_first_redemption_swaps_without_retirement_record() {
        let test = create_test_repo().await;
        let repo = &test.repo;
        let legacy_hash = session_contract::fixture_hash("lmdb-legacy:first-redemption");
        let base = Utc::now();

        let legacy = Session {
            refresh_token_hash: Secret::new(legacy_hash.clone()),
            family_id: String::new(),
            generation: 0,
            rotated_at: None,
            user_id: "usr_legacy".to_string(),
            provider: "google".to_string(),
            expires_at: base + chrono::Duration::hours(24),
            created_at: base,
            device_id: None,
            user_agent: None,
            ip_address: None,
        };
        repo.store_refresh_token(&legacy)
            .await
            .expect("store legacy row");

        match repo
            .resolve_refresh_token(&legacy_hash)
            .await
            .expect("resolve legacy")
        {
            RefreshResolution::Live(session) => {
                assert_eq!(session.family_id, "", "sentinel family on read");
                assert_eq!(session.generation, 0);
                assert_eq!(session.rotated_at, None);
            }
            other => panic!("the stored legacy row must resolve Live, got {other:?}"),
        }

        let new_family = fixture_family_id("lmdb-legacy:new-fam");
        assert!(is_valid_family_id(&new_family));
        let replacement = Session {
            refresh_token_hash: Secret::new(format!("{legacy_hash}-next")),
            family_id: new_family.clone(),
            ..legacy.clone()
        };

        let won = repo
            .rotate_refresh_token(&legacy_hash, &replacement)
            .await
            .expect("legacy first-redemption swap");
        assert!(won, "an uncontended legacy redemption must win its CAS");

        assert_eq!(
            repo.resolve_refresh_token(&legacy_hash)
                .await
                .expect("resolve consumed hash"),
            RefreshResolution::Unknown,
            "a consumed legacy row has no retained record and must read Unknown"
        );
        match repo
            .resolve_refresh_token(replacement.refresh_token_hash.expose())
            .await
            .expect("resolve replacement")
        {
            RefreshResolution::Live(session) => {
                assert_eq!(session.family_id, new_family);
                assert_eq!(session.user_id, "usr_legacy");
            }
            other => panic!("replacement must be Live, got {other:?}"),
        }

        let revoked = repo
            .revoke_family(&new_family)
            .await
            .expect("revoke new family");
        assert_eq!(
            revoked, 1,
            "only the replacement may exist for the new family - a legacy swap \
             must not have written a retirement record"
        );
    }

    /// Negative space: a losing CAS against a missing live generation writes
    /// nothing at all.
    #[tokio::test]
    async fn legacy_row_failed_cas_leaves_store_untouched() {
        let test = create_test_repo().await;
        let repo = &test.repo;
        let legacy_hash = session_contract::fixture_hash("lmdb-legacy:cas-failure");
        let base = Utc::now();

        let legacy = Session {
            refresh_token_hash: Secret::new(legacy_hash.clone()),
            family_id: String::new(),
            generation: 0,
            rotated_at: None,
            user_id: "usr_legacy".to_string(),
            provider: "google".to_string(),
            expires_at: base + chrono::Duration::hours(24),
            created_at: base,
            device_id: None,
            user_agent: None,
            ip_address: None,
        };
        repo.store_refresh_token(&legacy)
            .await
            .expect("store legacy row");

        let replacement = Session {
            refresh_token_hash: Secret::new(format!("{legacy_hash}-next")),
            family_id: fixture_family_id("lmdb-legacy:cas-fam"),
            ..legacy.clone()
        };

        let won = repo
            .rotate_refresh_token("no-such-live-hash", &replacement)
            .await
            .expect("rotation against an unknown live hash");
        assert!(!won, "a missing live generation must lose the CAS");
        assert!(
            repo.get_session_by_refresh_token(&Secret::new(legacy_hash.clone()))
                .await
                .expect("read legacy row")
                .is_some(),
            "the legacy row must survive the lost race"
        );
        assert_eq!(
            repo.resolve_refresh_token(replacement.refresh_token_hash.expose())
                .await
                .expect("resolve loser's proposal"),
            RefreshResolution::Unknown,
            "the loser's replacement must never be installed"
        );
    }

    /// Cleanup sweeps expired sessions *and* expired retirement records, its
    /// count covering both, leaving live state alone.
    #[tokio::test]
    async fn cleanup_counts_expired_sessions_and_retired_records_together() {
        let test = create_test_repo().await;
        let repo = &test.repo;
        let chain = family_chain("lmdb:cleanup", 0, "usr_cleanup");
        repo.store_refresh_token(&chain.gen0)
            .await
            .expect("store gen0");
        assert!(
            repo.rotate_refresh_token(chain.gen0.refresh_token_hash.expose(), &chain.gen1)
                .await
                .expect("rotate"),
            "rotation must win"
        );

        expire_everything_in_place(repo);

        let removed = repo.cleanup_expired_sessions().await.expect("cleanup");
        assert_eq!(
            removed, 2,
            "cleanup must count one expired session plus one expired retirement record"
        );
        assert_eq!(
            repo.count_active_sessions().await.expect("active count"),
            0,
            "nothing live remains after the sweep"
        );
        assert_eq!(
            repo.cleanup_expired_sessions()
                .await
                .expect("second cleanup"),
            0,
            "a second sweep over a clean store reports zero"
        );
    }

    /// A live session survives a cleanup that reaps its expired neighbours.
    #[tokio::test]
    async fn cleanup_leaves_live_sessions_alive() {
        let test = create_test_repo().await;
        let repo = &test.repo;
        let chain = family_chain("lmdb:cleanup-live", 0, "usr_mixed");
        repo.store_refresh_token(&chain.gen0)
            .await
            .expect("store live gen0");

        let dead_hash = session_contract::fixture_hash("lmdb:cleanup-live:dead");
        let dead = session_contract::generation_session(
            "usr_mixed",
            &fixture_family_id("lmdb:cleanup-live:dead-fam"),
            0,
            dead_hash.clone(),
            Utc::now() - chrono::Duration::hours(1),
            Utc::now(),
            None,
        );
        repo.store_refresh_token(&dead)
            .await
            .expect("store expired session");

        let removed = repo.cleanup_expired_sessions().await.expect("cleanup");
        assert_eq!(removed, 1, "exactly the expired neighbour is reaped");
        assert_eq!(
            repo.resolve_refresh_token(chain.gen0.refresh_token_hash.expose())
                .await
                .expect("resolve live session"),
            RefreshResolution::Live(chain.gen0),
            "the live session must survive the sweep"
        );
        assert_eq!(
            repo.resolve_refresh_token(&dead_hash)
                .await
                .expect("resolve dead session"),
            RefreshResolution::Unknown,
            "the expired session must be gone"
        );
    }

    /// `revoke_all_user_sessions` removes the user's live generations *and*
    /// their retained retirement records, leaving other users untouched.
    #[tokio::test]
    async fn revoke_all_user_sessions_sweeps_retired_records_of_that_user_only() {
        let test = create_test_repo().await;
        let repo = &test.repo;
        let mine = family_chain("lmdb:revoke-all", 0, "usr_mine");
        let theirs = family_chain("lmdb:revoke-all", 1, "usr_theirs");
        repo.store_refresh_token(&mine.gen0)
            .await
            .expect("store mine");
        repo.store_refresh_token(&theirs.gen0)
            .await
            .expect("store theirs");
        assert!(
            repo.rotate_refresh_token(mine.gen0.refresh_token_hash.expose(), &mine.gen1)
                .await
                .expect("rotate mine"),
            "my rotation must win"
        );
        assert!(
            repo.rotate_refresh_token(theirs.gen0.refresh_token_hash.expose(), &theirs.gen1)
                .await
                .expect("rotate theirs"),
            "their rotation must win"
        );

        repo.revoke_all_user_sessions("usr_mine")
            .await
            .expect("revoke all mine");

        assert_eq!(
            repo.resolve_refresh_token(mine.gen1.refresh_token_hash.expose())
                .await
                .expect("resolve my live"),
            RefreshResolution::Unknown,
            "my live generation must be gone"
        );
        assert_eq!(
            repo.resolve_refresh_token(mine.gen0.refresh_token_hash.expose())
                .await
                .expect("resolve my retired"),
            RefreshResolution::Unknown,
            "my retirement record must be gone"
        );
        match repo
            .resolve_refresh_token(theirs.gen1.refresh_token_hash.expose())
            .await
            .expect("resolve their live")
        {
            RefreshResolution::Live(session) => assert_eq!(session.user_id, "usr_theirs"),
            other => panic!("another user's family must survive my revocation, got {other:?}"),
        }
        // Their retirement record survives: revoking their family must still
        // find exactly the live generation plus the record.
        let revoked = repo
            .revoke_family(&theirs.family_id)
            .await
            .expect("revoke theirs");
        assert_eq!(
            revoked, 2,
            "the untouched family must keep both its live generation and its record"
        );
    }

    /// The high-occupancy regression behind [`LMDB_CLEANUP_BATCH_SIZE`]: on a
    /// map filled to near capacity with expired entries, the batched sweep
    /// completes (spawning several committed batches), reports every removal,
    /// and leaves the map writable again. A single-transaction sweep of the
    /// same occupancy fails `MDB_MAP_FULL`, which is the wedge this batching
    /// exists to prevent.
    #[tokio::test]
    async fn cleanup_reclaims_a_map_filled_near_capacity_in_batches() {
        let test = create_test_repo_with_map_mb(1).await;
        let repo = &test.repo;

        let mut seeded: usize = 0;
        for index in 0..FILL_MAX_ATTEMPTS {
            // Seed until the map crosses the near-capacity target (or an
            // insert already fails: a wedged-past-recovery map is outside the
            // sweeper's contract). Everything seeded is expired, so the sweep
            // has real work to do.
            if map_occupancy(repo) >= FILL_TARGET_FRACTION {
                break;
            }
            match seed_expired_session(repo, index).await {
                Ok(()) => seeded += 1,
                Err(Error::StoreError { detail }) if detail.contains("MDB_MAP_FULL") => break,
                Err(err) => panic!("unexpected failure while seeding: {err:?}"),
            }
        }
        assert!(
            seeded > LMDB_CLEANUP_BATCH_SIZE,
            "precondition: the fill loop must seed more than one batch worth \
             ({LMDB_CLEANUP_BATCH_SIZE}) of entries to prove batching, got {seeded}"
        );
        assert!(
            map_occupancy(repo) >= FILL_FLOOR_FRACTION,
            "precondition: the map must actually be near capacity, got {:.1}%",
            map_occupancy(repo) * 100.0
        );

        // The batched sweep must succeed on this near-capacity map — the
        // regime where wide delete transactions hit copy-on-write pressure —
        // and account for every expired entry. More removals than one batch
        // holds proves several committed write transactions (each freeing its
        // pages before the next begins), not one monolithic sweep; where the
        // pressure bites, the width degrades down to single-delete
        // transactions rather than failing.
        //
        // A deliberately monolithic control is intentionally absent: LMDB's
        // within-transaction page reuse lets even a whole-map sweep fit
        // whenever *any* headroom exists, so "monolithic fails here" is only
        // true at occupancy levels where no transaction of any width can run
        // — the documented grow-the-map boundary, not something a sweeper
        // regression can assert deterministically. What this test pins is the
        // property production relies on: high-occupancy sweeps complete in
        // bounded-size committed batches and leave the map writable.
        let removed = repo
            .cleanup_expired_sessions()
            .await
            .expect("the batched sweep must succeed on a near-full map");
        assert_eq!(
            removed as usize, seeded,
            "every seeded expired session must be reported as removed"
        );
        assert!(
            removed > LMDB_CLEANUP_BATCH_SIZE as u64,
            "more than one batch commit must have been exercised, removed={removed}"
        );

        // The freed pages are really back: a fresh live session stores and
        // classifies normally after the sweep.
        let survivor = session_contract::generation_session(
            "usr_after_fill",
            &fixture_family_id("lmdb-fill:after"),
            0,
            "after-cleanup-hash".to_string(),
            Utc::now() + chrono::Duration::hours(1),
            Utc::now(),
            None,
        );
        repo.store_refresh_token(&survivor)
            .await
            .expect("store after cleanup");
        assert_eq!(
            repo.count_active_sessions().await.expect("active count"),
            1,
            "exactly the post-sweep session remains active"
        );
    }
}

#[cfg(test)]
mod leak_span_tests_2 {
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
        // Field order is load-bearing: the subscriber guard must drop (uninstall)
        // BEFORE the gate releases, or the next capture's install races this
        // teardown of tracing's dispatcher registry.
        _guard: tracing::subscriber::DefaultGuard,
        _gate: std::sync::MutexGuard<'static, ()>,
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
        // Serialize capture tests process-wide (concurrent thread-local
        // subscriber installs race tracing's callsite interest cache), then
        // rebuild the cache under the installed subscriber.
        let gate = oidc_exchange_test_utils::telemetry::CAPTURE_GATE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let guard = tracing::subscriber::set_default(subscriber);
        tracing::callsite::rebuild_interest_cache();
        SpanCapture {
            _gate: gate,
            _guard: guard,
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
        let repo = LmdbSessionRepository::new(path.to_str().expect("utf-8 temp path"), 16, 3600)
            .expect("open lmdb environment");
        (repo, dir)
    }

    fn sentinel_session() -> Session {
        let now = Utc::now();
        Session {
            user_id: USER_ID_SENTINEL.to_string(),
            refresh_token_hash: Secret::new(HASH_SENTINEL.to_string()),
            family_id: oidc_exchange_core::domain::new_family_id(),
            generation: 0,
            rotated_at: None,
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
        // Capture liveness is a PRECONDITION here, not the property under test: under
        // parallel load, tracing's process-global callsite-interest cache can be
        // clobbered by concurrent callsite registration on gate-less threads, silently
        // suppressing a span mid-capture. A dead capture proves nothing either way, so
        // the ops are re-driven (fresh store, fresh capture) until the capture is
        // demonstrably live; the leak assertions then run against that live rendering.
        let mut live: Option<(String, SpanCapture)> = None;
        for _attempt in 0..5 {
            let capture = install_capture(SharedBuffer::default());
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

            // Non-vacuousness: all three instrumented spans must have both opened and
            // closed inside this capture before any absence claim means anything.
            let all_spans_rendered = [
                "store_refresh_token",
                "get_session_by_refresh_token",
                "revoke_session",
            ]
            .iter()
            .all(|span_name| rendered.matches(span_name).count() >= 2)
                && rendered.matches("close").count() == 3;
            if all_spans_rendered {
                live = Some((rendered, capture));
                break;
            }
        }
        let (rendered, capture) =
            live.expect("a live capture (all three spans open and close) within 5 attempts");

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
