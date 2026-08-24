use std::collections::HashMap;
use std::future::Future;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use oidc_exchange_core::domain::{
    is_valid_family_id, NewUser, RefreshResolution, RetiredRefreshToken, Session, User, UserPatch,
    UserStatus, INITIAL_USER_VERSION,
};
use serde_json::Value;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use tracing::instrument;

use oidc_exchange_core::error::{Error, Result};
use oidc_exchange_core::ports::{SessionRepository, UserRepository};

pub const MIGRATIONS: &str = r#"
CREATE TABLE IF NOT EXISTS users (
    id              TEXT PRIMARY KEY,
    external_id     TEXT NOT NULL,
    provider        TEXT NOT NULL,
    email           TEXT,
    display_name    TEXT,
    metadata        TEXT NOT NULL DEFAULT '{}',
    claims          TEXT NOT NULL DEFAULT '{}',
    status          TEXT NOT NULL DEFAULT 'active',
    version         INTEGER NOT NULL DEFAULT 1,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);

-- `(external_id, provider)` is unique only among live (non-deleted) users: a soft-deleted
-- user must free its identity for re-registration. `CREATE UNIQUE INDEX IF NOT EXISTS`
-- cannot turn a pre-existing full index into a partial one, so a database that predates
-- this migration needs the index dropped and recreated with the `WHERE` predicate. Safe to
-- re-run on every startup since both statements are idempotent (`IF EXISTS` / `IF NOT EXISTS`).
DROP INDEX IF EXISTS idx_users_external_id_provider;
CREATE UNIQUE INDEX IF NOT EXISTS idx_users_external_id_provider ON users(external_id, provider) WHERE status != 'deleted';

CREATE TABLE IF NOT EXISTS sessions (
    refresh_token_hash  TEXT PRIMARY KEY,
    user_id             TEXT NOT NULL,
    family_id           TEXT,
    generation          INTEGER NOT NULL DEFAULT 0,
    provider            TEXT NOT NULL,
    expires_at          TEXT NOT NULL,
    rotated_at          TEXT,
    device_id           TEXT,
    user_agent          TEXT,
    ip_address          TEXT,
    created_at          TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_sessions_user_id ON sessions(user_id);

CREATE TABLE IF NOT EXISTS retired_refresh_tokens (
    refresh_token_hash  TEXT PRIMARY KEY,
    family_id           TEXT NOT NULL,
    user_id             TEXT NOT NULL,
    successor_hash      TEXT NOT NULL,
    retired_at          TEXT NOT NULL,
    expires_at          TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_retired_family ON retired_refresh_tokens (family_id);
CREATE INDEX IF NOT EXISTS idx_retired_expires_at ON retired_refresh_tokens (expires_at);

-- Single-use records (nonces and assertion-replay markers): a presence-only digest key
-- plus an expiry. The `expires_at` index serves only the cleanup sweep — both claim
-- operations are keyed lookups that evaluate expiry themselves.
CREATE TABLE IF NOT EXISTS single_use (
    key         TEXT PRIMARY KEY,
    expires_at  TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_single_use_expires_at ON single_use(expires_at);
"#;

pub struct SqliteRepository {
    pool: SqlitePool,
    /// How long a retirement record stays readable after its rotation:
    /// `retired_at + reuse_retention_secs`, capped per record at the family's
    /// absolute `expires_at` by [`RetiredRefreshToken::retention_deadline`].
    /// Resolved from `[token] refresh_reuse_retention` at bootstrap; injected
    /// here because the store, not the caller, stamps every record's deadline.
    reuse_retention_secs: u64,
}

impl SqliteRepository {
    pub fn new(pool: SqlitePool, reuse_retention_secs: u64) -> Self {
        assert!(
            reuse_retention_secs > 0,
            "reuse retention must be greater than zero"
        );
        Self {
            pool,
            reuse_retention_secs,
        }
    }
}

/// Creates a SQLite connection pool, runs pragmas and migrations.
pub async fn create_pool(path: &str) -> std::result::Result<SqlitePool, Error> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await
        .map_err(|e| Error::StoreError {
            detail: e.to_string(),
        })?;

    // Run migrations
    sqlx::query(MIGRATIONS)
        .execute(&pool)
        .await
        .map_err(|e| Error::StoreError {
            detail: e.to_string(),
        })?;

    // `CREATE TABLE IF NOT EXISTS` above only covers fresh databases; a `users` table
    // that predates the `version` column needs an explicit, idempotent `ALTER TABLE`.
    ensure_version_column(&pool).await?;
    // Same for a `sessions` table that predates the session-family columns:
    // without them the row mapping in this module cannot round-trip a
    // `Session` (which now carries `family_id`/`generation`/`rotated_at`).
    ensure_session_family_columns(&pool).await?;

    Ok(pool)
}

/// Adds the `version` column (defaulting existing rows to [`INITIAL_USER_VERSION`]) to a
/// `users` table that predates it. Safe to call on every startup: it is a no-op once the
/// column exists, since SQLite's `ALTER TABLE … ADD COLUMN` has no `IF NOT EXISTS` form.
async fn ensure_version_column(pool: &SqlitePool) -> std::result::Result<(), Error> {
    let has_version =
        sqlx::query("SELECT 1 FROM pragma_table_info('users') WHERE name = 'version'")
            .fetch_optional(pool)
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?
            .is_some();

    if !has_version {
        sqlx::query(&format!(
            "ALTER TABLE users ADD COLUMN version INTEGER NOT NULL DEFAULT {INITIAL_USER_VERSION}"
        ))
        .execute(pool)
        .await
        .map_err(|e| Error::StoreError {
            detail: e.to_string(),
        })?;
    }

    Ok(())
}

/// The `sessions` columns introduced with refresh-token rotation, as
/// `(column name, ALTER TABLE statement)` pairs. Applied to a table that
/// predates rotation; each is a no-op once present (`pragma_table_info`
/// probe first — SQLite's `ADD COLUMN` has no `IF NOT EXISTS`). Legacy rows
/// get NULL `family_id`/`rotated_at` and generation 0.
const SESSION_FAMILY_COLUMNS: [(&str, &str); 3] = [
    (
        "family_id",
        "ALTER TABLE sessions ADD COLUMN family_id TEXT",
    ),
    (
        "generation",
        "ALTER TABLE sessions ADD COLUMN generation INTEGER NOT NULL DEFAULT 0",
    ),
    (
        "rotated_at",
        "ALTER TABLE sessions ADD COLUMN rotated_at TEXT",
    ),
];

/// Idempotently add [`SESSION_FAMILY_COLUMNS`] to a `sessions` table that
/// predates refresh-token rotation, mirroring [`ensure_version_column`].
async fn ensure_session_family_columns(pool: &SqlitePool) -> std::result::Result<(), Error> {
    for (column, ddl) in SESSION_FAMILY_COLUMNS {
        let has_column = sqlx::query("SELECT 1 FROM pragma_table_info('sessions') WHERE name = ?1")
            .bind(column)
            .fetch_optional(pool)
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?
            .is_some();
        if !has_column {
            sqlx::query(ddl)
                .execute(pool)
                .await
                .map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })?;
        }
    }
    Ok(())
}

/// SQLite extended result code for a unique-constraint violation
/// (`SQLITE_CONSTRAINT_UNIQUE`), per <https://www.sqlite.org/rescode.html#constraint_unique>.
const SQLITE_UNIQUE_VIOLATION_CODE: &str = "2067";

/// True when `err` is a SQLite unique-constraint violation, decided by the driver's
/// structured extended result code (not a substring match on the error message) so callers
/// can distinguish "already registered" from any other insert failure.
fn is_unique_violation(err: &sqlx::Error) -> bool {
    err.as_database_error()
        .and_then(|db_err| db_err.code())
        .is_some_and(|code| code == SQLITE_UNIQUE_VIOLATION_CODE)
}

/// Maximum number of read-modify-write attempts `update_user` makes against its
/// version-conditional `UPDATE … WHERE id = ?7 AND version = ?8` before giving up: the
/// initial attempt plus retries triggered by a losing race (zero rows affected because a
/// concurrent writer already advanced the row's `version`). Bounds retries so a row whose
/// `version` keeps changing under relentless concurrent writes cannot loop unbounded — it
/// errors instead of looping forever or silently overwriting the other writer's change.
const UPDATE_MAX_ATTEMPTS: u32 = 5;

/// Drives `update_user`'s version-conditional read-modify-write. Calls `attempt` (1-indexed)
/// up to `UPDATE_MAX_ATTEMPTS` times; `attempt` performs one full read-patch-write cycle and
/// returns `Ok(Some(user))` on a successful, version-conditioned write, or `Ok(None)` when
/// the write affected zero rows because a concurrent writer already advanced the row's
/// `version` (retry against the fresh value). Returns `Error::Conflict` — not an unbounded
/// loop — once the budget is exhausted without a successful write.
async fn retry_on_version_conflict<F, Fut>(user_id: &str, mut attempt: F) -> Result<User>
where
    F: FnMut(u32) -> Fut,
    Fut: Future<Output = Result<Option<User>>>,
{
    for attempt_number in 1..=UPDATE_MAX_ATTEMPTS {
        if let Some(user) = attempt(attempt_number).await? {
            return Ok(user);
        }
    }

    Err(Error::Conflict {
        detail: format!(
            "update_user for {user_id} exhausted the retry budget \
             ({UPDATE_MAX_ATTEMPTS} attempts) racing concurrent version conflicts"
        ),
    })
}

fn status_to_str(status: &UserStatus) -> &'static str {
    match status {
        UserStatus::Active => "active",
        UserStatus::Suspended => "suspended",
        UserStatus::Deleted => "deleted",
    }
}

fn str_to_status(s: &str) -> Result<UserStatus> {
    match s {
        "active" => Ok(UserStatus::Active),
        "suspended" => Ok(UserStatus::Suspended),
        "deleted" => Ok(UserStatus::Deleted),
        other => Err(Error::StoreError {
            detail: format!("unknown user status: {other}"),
        }),
    }
}

fn row_to_user(row: &sqlx::sqlite::SqliteRow) -> Result<User> {
    let metadata_str: String = row.get("metadata");
    let claims_str: String = row.get("claims");
    let status_str: String = row.get("status");
    let created_at_str: String = row.get("created_at");
    let updated_at_str: String = row.get("updated_at");

    let metadata: HashMap<String, Value> =
        serde_json::from_str(&metadata_str).map_err(|e| Error::StoreError {
            detail: format!("failed to parse metadata: {e}"),
        })?;
    let claims: HashMap<String, Value> =
        serde_json::from_str(&claims_str).map_err(|e| Error::StoreError {
            detail: format!("failed to parse claims: {e}"),
        })?;
    let created_at: DateTime<Utc> =
        created_at_str
            .parse()
            .map_err(|e: chrono::ParseError| Error::StoreError {
                detail: format!("failed to parse created_at: {e}"),
            })?;
    let updated_at: DateTime<Utc> =
        updated_at_str
            .parse()
            .map_err(|e: chrono::ParseError| Error::StoreError {
                detail: format!("failed to parse updated_at: {e}"),
            })?;

    Ok(User {
        id: row.get("id"),
        external_id: row.get("external_id"),
        provider: row.get("provider"),
        email: row.get("email"),
        display_name: row.get("display_name"),
        metadata,
        claims,
        status: str_to_status(&status_str)?,
        version: row.get::<i64, _>("version") as u64,
        created_at,
        updated_at,
    })
}

fn row_to_session(row: &sqlx::sqlite::SqliteRow) -> Result<Session> {
    let expires_at_str: String = row.get("expires_at");
    let created_at_str: String = row.get("created_at");

    let expires_at: DateTime<Utc> =
        expires_at_str
            .parse()
            .map_err(|e: chrono::ParseError| Error::StoreError {
                detail: format!("failed to parse expires_at: {e}"),
            })?;
    let created_at: DateTime<Utc> =
        created_at_str
            .parse()
            .map_err(|e: chrono::ParseError| Error::StoreError {
                detail: format!("failed to parse created_at: {e}"),
            })?;

    // A row written before rotation shipped has a NULL family_id (the columns
    // are added nullable by `ensure_session_family_columns`). The domain type
    // requires the field, so the sentinel is an *empty string* — deliberately
    // not a well-formed `fam_` id, so downstream family operations visibly
    // fail rather than silently matching a family that does not exist. The
    // rotation flow deletes such legacy rows on first redemption without
    // writing a retirement record.
    let family_id: Option<String> = row.get("family_id");

    let rotated_at: Option<DateTime<Utc>> = match row.get::<Option<String>, _>("rotated_at") {
        Some(s) => Some(
            s.parse()
                .map_err(|e: chrono::ParseError| Error::StoreError {
                    detail: format!("failed to parse rotated_at: {e}"),
                })?,
        ),
        None => None,
    };

    Ok(Session {
        user_id: row.get("user_id"),
        refresh_token_hash: row.get("refresh_token_hash"),
        family_id: family_id.unwrap_or_default(),
        generation: row.get::<i64, _>("generation") as u32,
        provider: row.get("provider"),
        expires_at,
        rotated_at,
        device_id: row.get("device_id"),
        user_agent: row.get("user_agent"),
        ip_address: row.get("ip_address"),
        created_at,
    })
}

fn parse_rfc3339(value: &str, field: &str) -> Result<DateTime<Utc>> {
    value
        .parse()
        .map_err(|e: chrono::ParseError| Error::StoreError {
            detail: format!("failed to parse {field}: {e}"),
        })
}

fn row_to_retired(row: &sqlx::sqlite::SqliteRow) -> Result<RetiredRefreshToken> {
    let retired_at_str: String = row.get("retired_at");
    let expires_at_str: String = row.get("expires_at");

    // Round-trip revalidation (store-read boundary): every column of a
    // retirement record is NOT NULL in the DDL, so an unparseable timestamp is
    // corruption, not absence, and must surface as a store error.
    Ok(RetiredRefreshToken {
        refresh_token_hash: row.get("refresh_token_hash"),
        family_id: row.get("family_id"),
        user_id: row.get("user_id"),
        successor_hash: row.get("successor_hash"),
        retired_at: parse_rfc3339(&retired_at_str, "retired_at")?,
        expires_at: parse_rfc3339(&expires_at_str, "expires_at")?,
    })
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
    assert_eq!(
        live.refresh_token_hash, live_hash,
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
        successor_hash: replacement.refresh_token_hash.clone(),
        retired_at: now,
        expires_at: RetiredRefreshToken::retention_deadline(
            now,
            reuse_retention_secs,
            replacement.expires_at,
        ),
    }
}

/// Insert one session row on an existing transaction or the pool. Shared by
/// `store_refresh_token` and the winning arm of `rotate_refresh_token` so
/// both write paths bind identical columns.
///
/// `replace` selects the conflict behaviour: the store path is idempotent
/// (`INSERT OR REPLACE`, preserving its existing semantics), while rotation
/// inserts plainly so a replacement hash colliding with any existing live row
/// fails the transaction loudly instead of clobbering it. Both fragments come
/// only from this module's own call sites, never from input.
async fn insert_session_tx<'e, E>(executor: E, session: &Session, replace: bool) -> Result<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let expires_at_str = session.expires_at.to_rfc3339();
    let created_at_str = session.created_at.to_rfc3339();
    let rotated_at_str = session.rotated_at.map(|ts| ts.to_rfc3339());

    let conflict_clause = if replace { "OR REPLACE" } else { "" };
    sqlx::query(
        &format!(
            "INSERT {conflict_clause} INTO sessions \
             (refresh_token_hash, user_id, family_id, generation, provider, expires_at, rotated_at, device_id, user_agent, ip_address, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"
        ),
    )
    .bind(&session.refresh_token_hash)
    .bind(&session.user_id)
    .bind(&session.family_id)
    .bind(session.generation as i64)
    .bind(&session.provider)
    .bind(&expires_at_str)
    .bind(&rotated_at_str)
    .bind(&session.device_id)
    .bind(&session.user_agent)
    .bind(&session.ip_address)
    .bind(&created_at_str)
    .execute(executor)
    .await
    .map_err(|e| Error::StoreError {
        detail: e.to_string(),
    })?;

    Ok(())
}

#[async_trait]
impl UserRepository for SqliteRepository {
    #[instrument(skip(self), fields(user_id))]
    async fn get_user_by_id(&self, user_id: &str) -> Result<Option<User>> {
        let row = sqlx::query("SELECT * FROM users WHERE id = ?1")
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

        match row {
            Some(ref r) => Ok(Some(row_to_user(r)?)),
            None => Ok(None),
        }
    }

    #[instrument(skip(self), fields(external_id, provider))]
    async fn get_user_by_external_id(
        &self,
        external_id: &str,
        provider: &str,
    ) -> Result<Option<User>> {
        // A soft-deleted user must never satisfy this lookup: deletion frees the
        // (provider, external_id) pair for re-registration (01-domain-model.md §Lifecycles).
        let row = sqlx::query(
            "SELECT * FROM users WHERE external_id = ?1 AND provider = ?2 AND status != 'deleted'",
        )
        .bind(external_id)
        .bind(provider)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::StoreError {
            detail: e.to_string(),
        })?;

        match row {
            Some(ref r) => Ok(Some(row_to_user(r)?)),
            None => Ok(None),
        }
    }

    #[instrument(skip(self, user), fields(external_id = %user.external_id, provider = %user.provider))]
    async fn create_user(&self, user: &NewUser) -> Result<User> {
        let now = Utc::now();
        let id = format!("usr_{}", ulid::Ulid::new().to_string().to_lowercase());
        let now_str = now.to_rfc3339();
        let metadata_str = "{}";
        let claims_str = "{}";
        let status_str = "active";

        sqlx::query(
            "INSERT INTO users (id, external_id, provider, email, display_name, metadata, claims, status, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        )
        .bind(&id)
        .bind(&user.external_id)
        .bind(&user.provider)
        .bind(&user.email)
        .bind(&user.display_name)
        .bind(metadata_str)
        .bind(claims_str)
        .bind(status_str)
        .bind(&now_str)
        .bind(&now_str)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            if is_unique_violation(&e) {
                Error::Conflict {
                    detail: format!(
                        "user already exists for external_id={} provider={}",
                        user.external_id, user.provider
                    ),
                }
            } else {
                Error::StoreError {
                    detail: e.to_string(),
                }
            }
        })?;

        Ok(User {
            id,
            external_id: user.external_id.clone(),
            provider: user.provider.clone(),
            email: user.email.clone(),
            display_name: user.display_name.clone(),
            metadata: HashMap::new(),
            claims: HashMap::new(),
            status: UserStatus::Active,
            version: INITIAL_USER_VERSION,
            created_at: now,
            updated_at: now,
        })
    }

    #[instrument(skip(self, patch), fields(user_id))]
    async fn update_user(&self, user_id: &str, patch: &UserPatch) -> Result<User> {
        retry_on_version_conflict(user_id, |attempt_number| async move {
            let mut user = self
                .get_user_by_id(user_id)
                .await?
                .ok_or_else(|| Error::StoreError {
                    detail: format!("user not found: {user_id}"),
                })?;
            let read_version = user.version as i64;

            if let Some(ref email) = patch.email {
                user.email = Some(email.clone());
            }
            if let Some(ref display_name) = patch.display_name {
                user.display_name = Some(display_name.clone());
            }
            if let Some(ref metadata) = patch.metadata {
                user.metadata = metadata.clone();
            }
            if let Some(ref claims) = patch.claims {
                user.claims = claims.clone();
            }
            if let Some(ref status) = patch.status {
                user.status = status.clone();
            }
            user.updated_at = Utc::now();
            user.version += 1;

            let metadata_str =
                serde_json::to_string(&user.metadata).map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })?;
            let claims_str = serde_json::to_string(&user.claims).map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;
            let status_str = status_to_str(&user.status);
            let updated_at_str = user.updated_at.to_rfc3339();

            // Version-conditional write: only the writer whose `read_version` still
            // matches the stored row wins, and it increments `version` in the same
            // statement. A concurrent writer that already bumped the row's version makes
            // this affect zero rows instead of silently clobbering the other writer's
            // change.
            let result = sqlx::query(
                "UPDATE users SET email = ?1, display_name = ?2, metadata = ?3, claims = ?4, status = ?5, updated_at = ?6, version = version + 1 \
                 WHERE id = ?7 AND version = ?8",
            )
            .bind(&user.email)
            .bind(&user.display_name)
            .bind(&metadata_str)
            .bind(&claims_str)
            .bind(status_str)
            .bind(&updated_at_str)
            .bind(user_id)
            .bind(read_version)
            .execute(&self.pool)
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

            if result.rows_affected() > 0 {
                return Ok(Some(user));
            }

            tracing::debug!(
                attempt = attempt_number,
                max_attempts = UPDATE_MAX_ATTEMPTS,
                "update_user version conflict, retrying"
            );
            Ok(None)
        })
        .await
    }

    #[instrument(skip(self), fields(user_id))]
    async fn delete_user(&self, user_id: &str) -> Result<()> {
        self.update_user(
            user_id,
            &UserPatch {
                email: None,
                display_name: None,
                metadata: None,
                claims: None,
                status: Some(UserStatus::Deleted),
            },
        )
        .await?;
        Ok(())
    }

    #[instrument(skip(self))]
    async fn count_by_status(&self) -> Result<HashMap<String, u64>> {
        let rows = sqlx::query("SELECT status, COUNT(*) as count FROM users GROUP BY status")
            .fetch_all(&self.pool)
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

        let mut counts = HashMap::new();
        for row in &rows {
            let status: String = row.get("status");
            let count: i64 = row.get("count");
            counts.insert(status, count as u64);
        }

        Ok(counts)
    }

    #[instrument(skip(self))]
    async fn list_users(&self, offset: u64, limit: u64) -> Result<Vec<User>> {
        let rows = sqlx::query("SELECT * FROM users ORDER BY created_at DESC LIMIT ?1 OFFSET ?2")
            .bind(limit as i64)
            .bind(offset as i64)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

        let mut users = Vec::new();
        for row in &rows {
            users.push(row_to_user(row)?);
        }

        Ok(users)
    }
}

#[async_trait]
impl SessionRepository for SqliteRepository {
    #[instrument(skip(self, session), fields(user_id = %session.user_id))]
    async fn store_refresh_token(&self, session: &Session) -> Result<()> {
        insert_session_tx(&self.pool, session, true).await
    }

    #[instrument(skip(self), fields(token_hash))]
    async fn get_session_by_refresh_token(&self, token_hash: &str) -> Result<Option<Session>> {
        let row = sqlx::query("SELECT * FROM sessions WHERE refresh_token_hash = ?1")
            .bind(token_hash)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

        match row {
            Some(ref r) => Ok(Some(row_to_session(r)?)),
            None => Ok(None),
        }
    }

    /// Classify `token_hash` against live generations first, then retained
    /// retirement records (SR1). Each read is its own committed query on this
    /// pool, and SQLite serializes writers, so the answer reflects the most
    /// recent write. A record past its retention deadline answers `Unknown`
    /// until the sweep physically deletes it — reuse detection must not fire
    /// on a window that has closed.
    #[instrument(skip(self), fields(token_hash))]
    async fn resolve_refresh_token(&self, token_hash: &str) -> Result<RefreshResolution> {
        assert!(
            !token_hash.is_empty(),
            "resolve_refresh_token: token_hash must not be empty"
        );
        if let Some(session) = self.get_session_by_refresh_token(token_hash).await? {
            return Ok(RefreshResolution::Live(session));
        }

        let Some(record) = self.get_retired_record(token_hash).await? else {
            return Ok(RefreshResolution::Unknown);
        };
        if record.expires_at <= Utc::now() {
            return Ok(RefreshResolution::Unknown);
        }

        match self
            .get_session_by_refresh_token(&record.successor_hash)
            .await?
        {
            Some(successor_live) => Ok(RefreshResolution::Superseded {
                live: successor_live,
                retired_at: record.retired_at,
            }),
            None => Ok(RefreshResolution::Retired {
                family_id: record.family_id,
                user_id: record.user_id,
                retired_at: record.retired_at,
            }),
        }
    }

    /// One `BEGIN … COMMIT` performing all rotation effects — delete the live
    /// row, write the retirement record, install the replacement — conditioned
    /// on the live row still existing (SR2/SR3/SR4). The condition is the
    /// delete's affected-row count: zero rows means a concurrent redemption
    /// moved the live generation first, so the transaction rolls back having
    /// written nothing and this returns `false`.
    ///
    /// A live row carrying the empty-family sentinel is a pre-rotation legacy
    /// row: its first redemption deletes it and installs the replacement
    /// *without* a retirement record — there is no prior generation to detect
    /// reuse against — and the replacement carries whatever family the caller
    /// minted. The store never invents one.
    #[instrument(skip(self, replacement), fields(token_hash = %live_hash, user_id = %replacement.user_id))]
    async fn rotate_refresh_token(&self, live_hash: &str, replacement: &Session) -> Result<bool> {
        assert_ne!(
            live_hash, replacement.refresh_token_hash,
            "rotate_refresh_token: replacement must be a fresh generation"
        );
        assert!(
            is_valid_family_id(&replacement.family_id),
            "rotate_refresh_token: malformed replacement family id {:?}",
            replacement.family_id
        );

        let mut tx = self.pool.begin().await.map_err(|e| Error::StoreError {
            detail: e.to_string(),
        })?;

        // Read the live row inside the transaction: its existence is the CAS
        // condition and its family/user identity decides between the normal
        // three-effect rotation and the legacy first-redemption swap.
        let live_row = sqlx::query("SELECT * FROM sessions WHERE refresh_token_hash = ?1")
            .bind(live_hash)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;
        let Some(live_row) = live_row else {
            // The condition failed: a concurrent redemption moved the live
            // generation first. Roll back and write nothing.
            tx.rollback().await.map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;
            return Ok(false);
        };
        let live = row_to_session(&live_row)?;
        let legacy_row = live.family_id.is_empty();
        if !legacy_row {
            // A rotation replaces a generation of the same family — anything
            // else would strand credentials outside their holder's control.
            assert_eq!(
                live.family_id, replacement.family_id,
                "rotate_refresh_token: family mismatch between live and replacement"
            );
        }
        assert_eq!(
            live.user_id, replacement.user_id,
            "rotate_refresh_token: user mismatch between live and replacement"
        );

        let deleted = sqlx::query("DELETE FROM sessions WHERE refresh_token_hash = ?1")
            .bind(live_hash)
            .execute(&mut *tx)
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;
        if deleted.rows_affected() == 0 {
            // The row the SELECT observed was concurrently removed and the
            // delete lost the race; the transaction must still roll back.
            tx.rollback().await.map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;
            return Ok(false);
        }

        if !legacy_row {
            let now = Utc::now();
            let record = retirement_record(
                live_hash,
                &live,
                replacement,
                self.reuse_retention_secs,
                now,
            );
            sqlx::query(
                "INSERT INTO retired_refresh_tokens (refresh_token_hash, family_id, user_id, successor_hash, retired_at, expires_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .bind(&record.refresh_token_hash)
            .bind(&record.family_id)
            .bind(&record.user_id)
            .bind(&record.successor_hash)
            .bind(record.retired_at.to_rfc3339())
            .bind(record.expires_at.to_rfc3339())
            .execute(&mut *tx)
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;
        }

        insert_session_tx(&mut *tx, replacement, false).await?;

        tx.commit().await.map_err(|e| Error::StoreError {
            detail: e.to_string(),
        })?;
        Ok(true)
    }

    #[instrument(skip(self), fields(token_hash))]
    async fn revoke_session(&self, token_hash: &str) -> Result<()> {
        sqlx::query("DELETE FROM sessions WHERE refresh_token_hash = ?1")
            .bind(token_hash)
            .execute(&self.pool)
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

        Ok(())
    }

    /// Remove the family's live generation and every retained retirement
    /// record, returning the combined count (SR5), inside one transaction so
    /// the two sweeps cannot be observed half-applied. Idempotent: an unknown
    /// (but well-formed) family id removes nothing and returns `Ok(0)`.
    #[instrument(skip(self), fields(family_id))]
    async fn revoke_family(&self, family_id: &str) -> Result<u64> {
        assert!(
            is_valid_family_id(family_id),
            "revoke_family: malformed family id {family_id:?}"
        );

        let mut tx = self.pool.begin().await.map_err(|e| Error::StoreError {
            detail: e.to_string(),
        })?;
        let live = sqlx::query("DELETE FROM sessions WHERE family_id = ?1")
            .bind(family_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;
        let retired = sqlx::query("DELETE FROM retired_refresh_tokens WHERE family_id = ?1")
            .bind(family_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;
        tx.commit().await.map_err(|e| Error::StoreError {
            detail: e.to_string(),
        })?;

        Ok(live.rows_affected() + retired.rows_affected())
    }

    #[instrument(skip(self))]
    async fn count_active_sessions(&self) -> Result<u64> {
        let now_str = Utc::now().to_rfc3339();
        let row = sqlx::query("SELECT COUNT(*) as count FROM sessions WHERE expires_at > ?1")
            .bind(&now_str)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

        let count: i64 = row.get("count");
        Ok(count as u64)
    }

    #[instrument(skip(self), fields(user_id))]
    async fn revoke_all_user_sessions(&self, user_id: &str) -> Result<()> {
        assert!(
            !user_id.is_empty(),
            "revoke_all_user_sessions: user_id must not be empty"
        );

        // The SR5 removal guarantee applied across all of the user's families:
        // live rows and retained retirement records leave in one transaction.
        let mut tx = self.pool.begin().await.map_err(|e| Error::StoreError {
            detail: e.to_string(),
        })?;
        sqlx::query("DELETE FROM sessions WHERE user_id = ?1")
            .bind(user_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;
        sqlx::query("DELETE FROM retired_refresh_tokens WHERE user_id = ?1")
            .bind(user_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;
        tx.commit().await.map_err(|e| Error::StoreError {
            detail: e.to_string(),
        })?;

        Ok(())
    }

    #[instrument(skip(self))]
    async fn cleanup_expired_sessions(&self) -> Result<u64> {
        let now_str = Utc::now().to_rfc3339();

        // The sweep covers sessions, retirement records, and expired
        // single-use records alike, and its count is the combined number
        // deleted (the port contract).
        let mut tx = self.pool.begin().await.map_err(|e| Error::StoreError {
            detail: e.to_string(),
        })?;
        let sessions = sqlx::query("DELETE FROM sessions WHERE expires_at < ?1")
            .bind(&now_str)
            .execute(&mut *tx)
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;
        let retired = sqlx::query("DELETE FROM retired_refresh_tokens WHERE expires_at < ?1")
            .bind(&now_str)
            .execute(&mut *tx)
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;
        let single_use = sqlx::query("DELETE FROM single_use WHERE expires_at < ?1")
            .bind(&now_str)
            .execute(&mut *tx)
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;
        tx.commit().await.map_err(|e| Error::StoreError {
            detail: e.to_string(),
        })?;

        Ok(sessions.rows_affected() + retired.rows_affected() + single_use.rows_affected())
    }

    #[instrument(skip(self, key))]
    async fn put_single_use(&self, key: &str, expires_at: DateTime<Utc>) -> Result<bool> {
        assert!(!key.is_empty(), "single-use key must be non-empty");

        // Insert-if-absent, with a live conflicting row overwritten only when it has
        // already expired (`WHERE single_use.expires_at < ?now`): rows_affected is 1 for
        // exactly the winning claim, 0 when a live record holds the key. The unique
        // primary key makes check-and-insert one atomic statement.
        let result = sqlx::query(
            "INSERT INTO single_use (key, expires_at) VALUES (?1, ?2) \
             ON CONFLICT(key) DO UPDATE SET expires_at = excluded.expires_at \
             WHERE single_use.expires_at < ?3",
        )
        .bind(key)
        .bind(expires_at.to_rfc3339())
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| Error::StoreError {
            detail: e.to_string(),
        })?;

        Ok(result.rows_affected() == 1)
    }

    #[instrument(skip(self, key))]
    async fn take_single_use(&self, key: &str) -> Result<bool> {
        assert!(!key.is_empty(), "single-use key must be non-empty");

        // Remove-and-report in one statement: only a live record (expiry still ahead of
        // now) matches, so an absent, burned, or expired key deletes zero rows.
        let row =
            sqlx::query("DELETE FROM single_use WHERE key = ?1 AND expires_at > ?2 RETURNING 1")
                .bind(key)
                .bind(Utc::now().to_rfc3339())
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })?;

        Ok(row.is_some())
    }
}

impl SqliteRepository {
    /// Fetch one retirement record by hash, if it is still retained. Inherent
    /// helper (not a port method): `resolve_refresh_token` needs the raw
    /// record to evaluate the successor pointer, while `/revoke`'s liveness
    /// lookup deliberately does not see retirement records as sessions.
    async fn get_retired_record(&self, token_hash: &str) -> Result<Option<RetiredRefreshToken>> {
        let row = sqlx::query("SELECT * FROM retired_refresh_tokens WHERE refresh_token_hash = ?1")
            .bind(token_hash)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

        match row {
            Some(ref r) => Ok(Some(row_to_retired(r)?)),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reuse-retention window used by every test repository: one hour — short
    /// enough that deadline arithmetic stays inside a test's lifetime, and
    /// positive per the constructor's precondition.
    const TEST_REUSE_RETENTION_SECS: u64 = 3600;

    async fn create_test_repo() -> SqliteRepository {
        let options = SqliteConnectOptions::new()
            .filename(":memory:")
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .foreign_keys(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .expect("failed to create in-memory pool");

        // SQLite doesn't support multiple statements in a single query call,
        // so we split the migrations and run them individually.
        for statement in MIGRATIONS.split(';') {
            let trimmed = statement.trim();
            if !trimmed.is_empty() {
                sqlx::query(trimmed)
                    .execute(&pool)
                    .await
                    .expect("failed to run migration statement");
            }
        }

        SqliteRepository::new(pool, TEST_REUSE_RETENTION_SECS)
    }

    #[tokio::test]
    async fn sqlite_user_crud() {
        let repo = create_test_repo().await;

        // Create user
        let new_user = NewUser {
            external_id: "google|user123".to_string(),
            provider: "google".to_string(),
            email: Some("alice@example.com".to_string()),
            display_name: Some("Alice".to_string()),
        };
        let created = repo.create_user(&new_user).await.expect("create_user");
        assert!(created.id.starts_with("usr_"));
        assert_eq!(created.external_id, "google|user123");
        assert_eq!(created.provider, "google");
        assert_eq!(created.email.as_deref(), Some("alice@example.com"));
        assert_eq!(created.status, UserStatus::Active);
        assert_eq!(created.version, INITIAL_USER_VERSION);

        // Get by ID
        let fetched = repo
            .get_user_by_id(&created.id)
            .await
            .expect("get_user_by_id")
            .expect("user should exist");
        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.version, INITIAL_USER_VERSION);

        // Get by external ID
        let fetched_ext = repo
            .get_user_by_external_id("google|user123", "google")
            .await
            .expect("get_user_by_external_id")
            .expect("user should exist");
        assert_eq!(fetched_ext.id, created.id);

        // Get non-existent
        let none = repo
            .get_user_by_id("usr_nonexistent")
            .await
            .expect("get_user_by_id");
        assert!(none.is_none());

        // Update user
        let patch = UserPatch {
            email: Some("alice-new@example.com".to_string()),
            display_name: None,
            metadata: Some({
                let mut m = HashMap::new();
                m.insert("key".to_string(), Value::String("val".to_string()));
                m
            }),
            claims: None,
            status: None,
        };
        let updated = repo
            .update_user(&created.id, &patch)
            .await
            .expect("update_user");
        assert_eq!(updated.email.as_deref(), Some("alice-new@example.com"));
        assert_eq!(
            updated.metadata.get("key"),
            Some(&Value::String("val".to_string()))
        );

        // Verify update persisted
        let re_fetched = repo
            .get_user_by_id(&created.id)
            .await
            .expect("get_user_by_id")
            .expect("user should exist");
        assert_eq!(re_fetched.email.as_deref(), Some("alice-new@example.com"));

        // Delete (soft)
        repo.delete_user(&created.id).await.expect("delete_user");
        let deleted = repo
            .get_user_by_id(&created.id)
            .await
            .expect("get_user_by_id")
            .expect("user should still exist");
        assert_eq!(deleted.status, UserStatus::Deleted);
    }

    #[tokio::test]
    async fn sqlite_session_crud() {
        let repo = create_test_repo().await;

        let now = Utc::now();
        let session = Session {
            user_id: "usr_test123".to_string(),
            refresh_token_hash: "hash_abc123".to_string(),
            family_id: "fam_0000000000000000000000000a".to_string(),
            generation: 0,
            provider: "google".to_string(),
            expires_at: now + chrono::Duration::hours(24),
            rotated_at: None,
            device_id: Some("device-1".to_string()),
            user_agent: Some("test-agent".to_string()),
            ip_address: Some("10.0.0.1".to_string()),
            created_at: now,
        };

        // Store
        repo.store_refresh_token(&session)
            .await
            .expect("store_refresh_token");

        // Get
        let fetched = repo
            .get_session_by_refresh_token("hash_abc123")
            .await
            .expect("get_session")
            .expect("session should exist");
        assert_eq!(fetched.user_id, "usr_test123");
        assert_eq!(fetched.device_id.as_deref(), Some("device-1"));

        // Non-existent
        let none = repo
            .get_session_by_refresh_token("hash_nonexistent")
            .await
            .expect("get_session");
        assert!(none.is_none());

        // Store second session
        let session2 = Session {
            user_id: "usr_test123".to_string(),
            refresh_token_hash: "hash_def456".to_string(),
            family_id: "fam_0000000000000000000000000b".to_string(),
            generation: 0,
            provider: "google".to_string(),
            expires_at: now + chrono::Duration::hours(24),
            rotated_at: None,
            device_id: None,
            user_agent: None,
            ip_address: None,
            created_at: now,
        };
        repo.store_refresh_token(&session2)
            .await
            .expect("store second session");

        // Revoke single
        repo.revoke_session("hash_abc123")
            .await
            .expect("revoke_session");
        assert!(repo
            .get_session_by_refresh_token("hash_abc123")
            .await
            .expect("get")
            .is_none());
        assert!(repo
            .get_session_by_refresh_token("hash_def456")
            .await
            .expect("get")
            .is_some());

        // Re-store first, then revoke all
        repo.store_refresh_token(&session).await.expect("re-store");
        repo.revoke_all_user_sessions("usr_test123")
            .await
            .expect("revoke_all");
        assert!(repo
            .get_session_by_refresh_token("hash_abc123")
            .await
            .expect("get")
            .is_none());
        assert!(repo
            .get_session_by_refresh_token("hash_def456")
            .await
            .expect("get")
            .is_none());
    }

    #[tokio::test]
    async fn create_pool_runs_migrations_and_round_trips_version() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("oidc.sqlite3");
        let path_str = path.to_str().expect("utf8 path").to_string();

        let pool = create_pool(&path_str).await.expect("create_pool");
        let repo = SqliteRepository::new(pool, TEST_REUSE_RETENTION_SECS);

        let created = repo
            .create_user(&NewUser {
                external_id: "google|pooltest".to_string(),
                provider: "google".to_string(),
                email: Some("pool@example.com".to_string()),
                display_name: None,
            })
            .await
            .expect("create_user via real create_pool");
        assert_eq!(created.version, INITIAL_USER_VERSION);

        let fetched = repo
            .get_user_by_id(&created.id)
            .await
            .expect("get_user_by_id")
            .expect("user should exist");
        assert_eq!(fetched.version, created.version);
    }

    /// Negative-space: a `users` row written before the `version` column existed must
    /// still read back as [`INITIAL_USER_VERSION`] once the idempotent migration runs.
    #[tokio::test]
    async fn legacy_row_without_version_column_defaults_to_initial_version() {
        let options = SqliteConnectOptions::new()
            .filename(":memory:")
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .expect("failed to create in-memory pool");

        // Simulate a pre-migration `users` table: no `version` column.
        sqlx::query(
            "CREATE TABLE users (
                id              TEXT PRIMARY KEY,
                external_id     TEXT NOT NULL,
                provider        TEXT NOT NULL,
                email           TEXT,
                display_name    TEXT,
                metadata        TEXT NOT NULL DEFAULT '{}',
                claims          TEXT NOT NULL DEFAULT '{}',
                status          TEXT NOT NULL DEFAULT 'active',
                created_at      TEXT NOT NULL,
                updated_at      TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .expect("create legacy users table");

        let now_str = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO users (id, external_id, provider, status, created_at, updated_at) \
             VALUES (?1, ?2, ?3, 'active', ?4, ?4)",
        )
        .bind("usr_legacy")
        .bind("legacy-ext")
        .bind("google")
        .bind(&now_str)
        .execute(&pool)
        .await
        .expect("insert legacy row");

        ensure_version_column(&pool)
            .await
            .expect("migration should add the version column");

        let repo = SqliteRepository::new(pool, TEST_REUSE_RETENTION_SECS);
        let user = repo
            .get_user_by_id("usr_legacy")
            .await
            .expect("get_user_by_id")
            .expect("legacy user should be found");
        assert_eq!(user.version, INITIAL_USER_VERSION);
        assert_eq!(user.external_id, "legacy-ext");
    }

    #[tokio::test]
    async fn ensure_version_column_is_idempotent() {
        let repo = create_test_repo().await;

        ensure_version_column(&repo.pool)
            .await
            .expect("first call should succeed (column already present)");
        ensure_version_column(&repo.pool)
            .await
            .expect("second call should also succeed");

        let version_columns =
            sqlx::query("SELECT name FROM pragma_table_info('users') WHERE name = 'version'")
                .fetch_all(&repo.pool)
                .await
                .expect("query table info");
        assert_eq!(
            version_columns.len(),
            1,
            "version column should not be duplicated by repeated migration runs"
        );
    }

    #[tokio::test]
    async fn create_user_duplicate_external_id_returns_conflict() {
        let repo = create_test_repo().await;

        let new_user = NewUser {
            external_id: "google|dup_test".to_string(),
            provider: "google".to_string(),
            email: Some("dup@example.com".to_string()),
            display_name: None,
        };
        repo.create_user(&new_user)
            .await
            .expect("first create_user should succeed");

        let err = repo
            .create_user(&new_user)
            .await
            .expect_err("second create_user with the same (external_id, provider) should fail");
        match err {
            Error::Conflict { detail } => {
                assert!(
                    !detail.is_empty(),
                    "Conflict detail should describe the collision"
                );
            }
            other => panic!("expected Error::Conflict, got {other:?}"),
        }
    }

    /// Deletion frees `(provider, external_id)` for re-registration: `get_user_by_external_id`
    /// stops returning the deleted row, `create_user` succeeds as a brand-new user rather than
    /// conflicting, and the soft-deleted row is retained. Negative-space at the end: a further
    /// live duplicate against the recreated user must still conflict — deletion frees the id,
    /// it does not disable uniqueness among live rows.
    #[tokio::test]
    async fn delete_user_frees_external_id_for_recreation() {
        let repo = create_test_repo().await;

        let new_user = NewUser {
            external_id: "google|sqlite_delete_frees_test".to_string(),
            provider: "google".to_string(),
            email: Some("first@example.com".to_string()),
            display_name: None,
        };
        let original = repo.create_user(&new_user).await.expect("create_user");

        repo.delete_user(&original.id).await.expect("delete_user");

        // A deleted user must not satisfy the external-id lookup.
        let looked_up = repo
            .get_user_by_external_id(&new_user.external_id, &new_user.provider)
            .await
            .expect("get_user_by_external_id");
        assert!(
            looked_up.is_none(),
            "deleted user must not be returned by external-id lookup"
        );

        // The identity is free: create_user for the same (provider, external_id) succeeds
        // as a brand-new user, not a conflict.
        let recreated = repo
            .create_user(&new_user)
            .await
            .expect("recreate after delete should succeed, not conflict");
        assert_ne!(
            recreated.id, original.id,
            "recreated user must get a fresh id"
        );
        assert!(
            recreated.claims.is_empty(),
            "recreated user must start with no carried-over claims"
        );

        // The original (soft-deleted) row is retained, not purged.
        let original_row = repo
            .get_user_by_id(&original.id)
            .await
            .expect("get_user_by_id")
            .expect("deleted row should still exist");
        assert_eq!(original_row.status, UserStatus::Deleted);

        // Negative-space: a second live duplicate against the recreated user still
        // conflicts — deletion frees the id, it does not disable uniqueness among live rows.
        let err = repo
            .create_user(&new_user)
            .await
            .expect_err("a second live duplicate must still conflict");
        match err {
            Error::Conflict { .. } => {}
            other => panic!("expected Error::Conflict, got {other:?}"),
        }
    }

    /// The partial-index migration must upgrade a database that predates it (a full unique
    /// index across all rows, deleted or not) by dropping and recreating the index with the
    /// `WHERE status != 'deleted'` predicate, and must be safe to re-run more than once.
    #[tokio::test]
    async fn partial_unique_index_migration_upgrades_legacy_full_index_and_is_idempotent() {
        let repo = create_test_repo().await;

        // Simulate a legacy database: replace the partial index the test-repo bootstrap
        // just created with the old full unique index that predates this migration
        // (uniqueness enforced across *all* rows, deleted or not).
        sqlx::query("DROP INDEX IF EXISTS idx_users_external_id_provider")
            .execute(&repo.pool)
            .await
            .expect("drop partial index to simulate a legacy schema");
        sqlx::query(
            "CREATE UNIQUE INDEX idx_users_external_id_provider ON users(external_id, provider)",
        )
        .execute(&repo.pool)
        .await
        .expect("recreate legacy full unique index");

        // Re-running the migration DDL (statement by statement, as `create_pool` and this
        // test harness both do for SQLite) must drop the legacy full index and recreate it
        // as partial — and running it a second time must not error (idempotent).
        for _ in 0..2 {
            for statement in MIGRATIONS.split(';') {
                let trimmed = statement.trim();
                if !trimmed.is_empty() {
                    sqlx::query(trimmed)
                        .execute(&repo.pool)
                        .await
                        .expect("re-running a migration statement should not error");
                }
            }
        }

        let new_user = NewUser {
            external_id: "google|sqlite_migration_test".to_string(),
            provider: "google".to_string(),
            email: None,
            display_name: None,
        };
        let created = repo.create_user(&new_user).await.expect("create_user");
        repo.delete_user(&created.id).await.expect("delete_user");

        // Under the legacy full index this would still conflict; under the partial index
        // it must succeed, since the deleted row no longer occupies the
        // (external_id, provider) slot.
        let recreated = repo
            .create_user(&new_user)
            .await
            .expect("recreate after delete should succeed under the partial index");
        assert_ne!(recreated.id, created.id);
    }

    /// Negative-space: an insert failure that is *not* a unique-constraint violation must
    /// still map to `Error::StoreError`, not be misclassified as `Conflict`.
    #[tokio::test]
    async fn create_user_non_unique_failure_maps_to_store_error() {
        let repo = create_test_repo().await;

        // Drop the table out from under `create_user` so the insert fails for a reason
        // other than a unique-constraint violation ("no such table", SQLite primary
        // result code `SQLITE_ERROR`).
        sqlx::query("DROP TABLE users")
            .execute(&repo.pool)
            .await
            .expect("drop users table");

        let err = repo
            .create_user(&NewUser {
                external_id: "google|store_error_test".to_string(),
                provider: "google".to_string(),
                email: None,
                display_name: None,
            })
            .await
            .expect_err("insert against a missing table should fail");
        match err {
            Error::StoreError { detail } => {
                assert!(
                    !detail.is_empty(),
                    "StoreError detail should describe the failure"
                );
            }
            other => panic!("expected Error::StoreError, got {other:?}"),
        }
    }

    /// [`is_unique_violation`] must decide from the driver's structured extended result
    /// code, not a substring of the error message: a genuine unique-constraint violation
    /// reads `true` off code `2067`, and a differently-coded failure (here, a `NOT NULL`
    /// violation) reads `false`.
    #[tokio::test]
    async fn is_unique_violation_reads_structured_code_not_message() {
        let repo = create_test_repo().await;

        let now_str = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO users (id, external_id, provider, status, created_at, updated_at) \
             VALUES (?1, ?2, ?3, 'active', ?4, ?4)",
        )
        .bind("usr_unique_probe")
        .bind("google|unique_probe")
        .bind("google")
        .bind(&now_str)
        .execute(&repo.pool)
        .await
        .expect("seed row for unique-violation probe");

        let unique_violation_err = sqlx::query(
            "INSERT INTO users (id, external_id, provider, status, created_at, updated_at) \
             VALUES (?1, ?2, ?3, 'active', ?4, ?4)",
        )
        .bind("usr_unique_probe_2")
        .bind("google|unique_probe")
        .bind("google")
        .bind(&now_str)
        .execute(&repo.pool)
        .await
        .expect_err("duplicate (external_id, provider) insert should fail");
        assert!(
            is_unique_violation(&unique_violation_err),
            "a genuine unique-constraint violation should be classified as such"
        );
        let code = unique_violation_err
            .as_database_error()
            .and_then(|e| e.code())
            .expect("database error should carry a structured code");
        assert_eq!(code, SQLITE_UNIQUE_VIOLATION_CODE);

        let not_null_violation_err = sqlx::query(
            "INSERT INTO users (id, external_id, provider, status, created_at, updated_at) \
             VALUES (?1, NULL, ?2, 'active', ?3, ?3)",
        )
        .bind("usr_not_null_probe")
        .bind("google")
        .bind(&now_str)
        .execute(&repo.pool)
        .await
        .expect_err("NULL into a NOT NULL column should fail");
        assert!(
            !is_unique_violation(&not_null_violation_err),
            "a NOT NULL violation must not be misclassified as a unique violation"
        );
    }

    /// A [`SqliteRepository`] backed by a file (not `:memory:`) with a multi-connection
    /// pool, so two callers can genuinely race concurrent writes against the same row —
    /// an in-memory, single-connection pool serializes every query at the connection level
    /// and cannot exercise `update_user`'s version-conflict retry path. The returned
    /// `TempDir` must be kept alive for the database file to remain in place.
    async fn create_racing_test_repo() -> (SqliteRepository, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("racing.sqlite3");
        let path_str = path.to_str().expect("utf8 path").to_string();

        let options = SqliteConnectOptions::new()
            .filename(&path_str)
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .foreign_keys(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await
            .expect("failed to create file-backed pool");

        for statement in MIGRATIONS.split(';') {
            let trimmed = statement.trim();
            if !trimmed.is_empty() {
                sqlx::query(trimmed)
                    .execute(&pool)
                    .await
                    .expect("failed to run migration statement");
            }
        }

        (SqliteRepository::new(pool, TEST_REUSE_RETENTION_SECS), dir)
    }

    #[tokio::test]
    async fn racing_suspend_and_claims_patch_ends_suspended() {
        let (repo, _dir) = create_racing_test_repo().await;

        let created = repo
            .create_user(&NewUser {
                external_id: "google|sqlite_version_race_test".to_string(),
                provider: "google".to_string(),
                email: Some("race@example.com".to_string()),
                display_name: None,
            })
            .await
            .expect("create_user");
        assert_eq!(created.version, INITIAL_USER_VERSION);

        let suspend_patch = UserPatch {
            email: None,
            display_name: None,
            metadata: None,
            claims: None,
            status: Some(UserStatus::Suspended),
        };
        let mut claims = HashMap::new();
        claims.insert(
            "org_id".to_string(),
            Value::String("org_racing".to_string()),
        );
        let claims_patch = UserPatch {
            email: None,
            display_name: None,
            metadata: None,
            claims: Some(claims),
            status: None,
        };

        // Race a suspend patch against an unrelated claims patch, both reading the same
        // starting `version`. The version-conditional write lets exactly one land per
        // attempt; the other retries against the fresh version until it too succeeds.
        let (suspend_result, claims_result) = tokio::join!(
            repo.update_user(&created.id, &suspend_patch),
            repo.update_user(&created.id, &claims_patch),
        );

        suspend_result.expect("suspend patch should eventually succeed");
        claims_result.expect("claims patch should eventually succeed");

        let final_user = repo
            .get_user_by_id(&created.id)
            .await
            .expect("get_user_by_id")
            .expect("user should exist");

        // Both racing writes landed — neither silently reverted the other — and `version`
        // advanced by exactly one per successful write.
        assert_eq!(final_user.status, UserStatus::Suspended);
        assert_eq!(
            final_user.claims.get("org_id"),
            Some(&Value::String("org_racing".to_string()))
        );
        assert_eq!(final_user.version, INITIAL_USER_VERSION + 2);
    }

    /// Negative-space: when every attempt's version-conditioned write loses the race (the
    /// row's `version` "keeps changing" out from under it), `retry_on_version_conflict`
    /// must exhaust `UPDATE_MAX_ATTEMPTS` and return `Error::Conflict` — not loop unbounded
    /// or silently report success. Exercises the retry driver directly with a closure that
    /// always reports a conflict, the same technique [`drain_unprocessed`]-style retry
    /// loops elsewhere in this codebase use to make budget exhaustion deterministically
    /// testable without a live, racing database.
    #[tokio::test]
    async fn retry_on_version_conflict_errors_when_every_attempt_conflicts() {
        let calls = std::sync::atomic::AtomicU32::new(0);

        let result = retry_on_version_conflict("usr_relentless", |attempt_number| {
            calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            assert!(
                (1..=UPDATE_MAX_ATTEMPTS).contains(&attempt_number),
                "attempt numbers should stay within the bound"
            );
            std::future::ready(Ok(None))
        })
        .await;

        match result {
            Err(Error::Conflict { detail }) => {
                assert!(
                    detail.contains("usr_relentless") && detail.contains("exhausted"),
                    "error should name the user and explain budget exhaustion: {detail}"
                );
            }
            other => panic!("expected Error::Conflict on retry-budget exhaustion, got {other:?}"),
        }
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::Relaxed),
            UPDATE_MAX_ATTEMPTS,
            "should make exactly UPDATE_MAX_ATTEMPTS attempts, no more and no fewer"
        );
    }

    /// The mirror-image happy path: a conflict on the first attempts must not abort the
    /// retry — it should keep trying and return the eventual success once the write stops
    /// losing the race, well within the budget.
    #[tokio::test]
    async fn retry_on_version_conflict_succeeds_once_a_later_attempt_wins() {
        let calls = std::sync::atomic::AtomicU32::new(0);
        let winning_attempt = 3u32;

        let result = retry_on_version_conflict("usr_eventually_wins", |attempt_number| {
            calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            std::future::ready(if attempt_number == winning_attempt {
                Ok(Some(User {
                    id: "usr_eventually_wins".to_string(),
                    external_id: "google|eventual".to_string(),
                    provider: "google".to_string(),
                    email: None,
                    display_name: None,
                    metadata: HashMap::new(),
                    claims: HashMap::new(),
                    status: UserStatus::Active,
                    version: attempt_number as u64 + 1,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                }))
            } else {
                Ok(None)
            })
        })
        .await
        .expect("should eventually succeed within the budget");

        assert_eq!(result.id, "usr_eventually_wins");
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::Relaxed),
            winning_attempt,
            "should stop retrying as soon as an attempt succeeds, not keep spinning"
        );
    }

    // -----------------------------------------------------------------------
    // Refresh-token rotation and reuse detection (task 03)
    // -----------------------------------------------------------------------

    /// Insert a pre-rotation (legacy) session row exactly the way the
    /// pre-migration schema would have written it: NULL `family_id`,
    /// generation 0, NULL `rotated_at`.
    async fn seed_legacy_session(repo: &SqliteRepository, token_hash: &str, user_id: &str) {
        let now_str = Utc::now().to_rfc3339();
        let expires_str = (Utc::now() + chrono::Duration::hours(24)).to_rfc3339();
        sqlx::query(
            "INSERT INTO sessions (refresh_token_hash, user_id, family_id, generation, provider, expires_at, rotated_at, device_id, user_agent, ip_address, created_at) \
             VALUES (?1, ?2, NULL, 0, 'google', ?3, NULL, NULL, NULL, NULL, ?4)",
        )
        .bind(token_hash)
        .bind(user_id)
        .bind(expires_str)
        .bind(now_str)
        .execute(&repo.pool)
        .await
        .expect("seed legacy session row");
    }

    async fn retired_count(repo: &SqliteRepository) -> i64 {
        let row = sqlx::query("SELECT COUNT(*) AS count FROM retired_refresh_tokens")
            .fetch_one(&repo.pool)
            .await
            .expect("count retirement records");
        row.get::<i64, _>("count")
    }

    /// The full SR1–SR5 shared suite against the SQLite store. One tag
    /// namespaces every fixture the suite creates.
    #[tokio::test]
    async fn sqlite_session_store_meets_sr1_through_sr5() {
        let repo = create_test_repo().await;
        oidc_exchange_test_utils::session_contract::assert_full_conformance(
            &repo,
            "sqlite-session-conformance",
        )
        .await;
    }

    /// A legacy row's first redemption swaps atomically but writes no
    /// retirement record — there is no prior generation to detect reuse
    /// against — and the presented hash reads Unknown afterwards. The
    /// replacement carries the caller's newly-minted family; nothing here
    /// synthesizes one.
    #[tokio::test]
    async fn legacy_row_first_redemption_swaps_without_retirement_record() {
        let repo = create_test_repo().await;
        let legacy_hash = oidc_exchange_test_utils::session_contract::fixture_hash(
            "sqlite-legacy:first-redemption",
        );
        seed_legacy_session(&repo, &legacy_hash, "usr_legacy").await;

        // Classification is storage-factual: the sentinel-carrying row is Live.
        let legacy = repo
            .get_session_by_refresh_token(&legacy_hash)
            .await
            .expect("read legacy row")
            .expect("legacy row must exist");
        assert_eq!(legacy.family_id, "", "sentinel family on read");
        assert_eq!(legacy.generation, 0);
        assert_eq!(legacy.rotated_at, None);

        let base = Utc::now();
        let new_family =
            oidc_exchange_test_utils::session_contract::fixture_family_id("sqlite-legacy:new-fam");
        assert!(is_valid_family_id(&new_family));
        let replacement = Session {
            refresh_token_hash: format!("{legacy_hash}-next"),
            family_id: new_family.clone(),
            generation: 0,
            rotated_at: None,
            expires_at: base + chrono::Duration::hours(24),
            created_at: base,
            ..legacy.clone()
        };

        let won = repo
            .rotate_refresh_token(&legacy_hash, &replacement)
            .await
            .expect("legacy first-redemption swap");
        assert!(won, "an uncontended legacy redemption must win its CAS");

        use oidc_exchange_core::domain::RefreshResolution::*;
        assert_eq!(
            repo.resolve_refresh_token(&legacy_hash)
                .await
                .expect("resolve"),
            Unknown,
            "a consumed legacy row has no retained record and must read Unknown"
        );
        match repo
            .resolve_refresh_token(&replacement.refresh_token_hash)
            .await
            .expect("resolve replacement")
        {
            Live(session) => {
                assert_eq!(session.family_id, new_family);
                assert_eq!(session.user_id, "usr_legacy");
            }
            other => panic!("replacement must be Live, got {other:?}"),
        }
        assert_eq!(
            retired_count(&repo).await,
            0,
            "a legacy first redemption must not leave a retirement record"
        );
        assert_eq!(
            repo.count_active_sessions().await.expect("active count"),
            1,
            "exactly the replacement is active after the swap"
        );
    }

    /// Negative space: a losing CAS against a legacy row writes nothing at all.
    #[tokio::test]
    async fn legacy_row_failed_cas_leaves_store_untouched() {
        let repo = create_test_repo().await;
        let legacy_hash =
            oidc_exchange_test_utils::session_contract::fixture_hash("sqlite-legacy:cas-failure");
        seed_legacy_session(&repo, &legacy_hash, "usr_legacy").await;

        let base = Utc::now();
        let replacement = Session {
            refresh_token_hash: format!("{legacy_hash}-next"),
            family_id: oidc_exchange_test_utils::session_contract::fixture_family_id(
                "sqlite-legacy:cas-fam",
            ),
            user_id: "usr_legacy".to_string(),
            provider: "google".to_string(),
            expires_at: base + chrono::Duration::hours(24),
            rotated_at: None,
            device_id: None,
            user_agent: None,
            ip_address: None,
            created_at: base,
            generation: 0,
        };

        let won = repo
            .rotate_refresh_token("no-such-live-hash", &replacement)
            .await
            .expect("rotation against an unknown live hash");
        assert!(!won, "a missing live generation must lose the CAS");
        assert_eq!(
            repo.get_session_by_refresh_token(&legacy_hash)
                .await
                .expect("read legacy row")
                .expect("legacy row must survive the lost race")
                .refresh_token_hash,
            legacy_hash,
        );
        assert_eq!(
            retired_count(&repo).await,
            0,
            "no retirement record may appear when the CAS fails"
        );
        assert!(
            repo.get_session_by_refresh_token(&replacement.refresh_token_hash)
                .await
                .expect("read replacement")
                .is_none(),
            "the loser's replacement must never be installed"
        );
    }

    /// Transaction rollback: when the replacement insert fails mid-transaction
    /// (its hash colliding with an existing live row), the whole unit rolls
    /// back — the live generation the delete removed comes back, and no
    /// orphaned retirement record is left behind (SR2's all-or-nothing rule
    /// under a real failure, not just a lost CAS).
    #[tokio::test]
    async fn failed_replacement_insert_rolls_back_delete_and_retirement() {
        let repo = create_test_repo().await;
        use oidc_exchange_test_utils::session_contract::{family_chain, fixture_family_id};

        let chain = family_chain("sqlite:rollback", 0, "usr_rollback");
        repo.store_refresh_token(&chain.gen0)
            .await
            .expect("store gen0");

        // An unrelated live row already occupying the replacement's hash: the
        // mid-transaction collision that forces the rollback.
        let blocker = Session {
            refresh_token_hash: chain.gen1.refresh_token_hash.clone(),
            family_id: fixture_family_id("sqlite:rollback:blocker-fam"),
            user_id: "usr_blocker".to_string(),
            ..chain.gen1.clone()
        };
        repo.store_refresh_token(&blocker)
            .await
            .expect("store blocker");

        let result = repo
            .rotate_refresh_token(&chain.gen0.refresh_token_hash, &chain.gen1)
            .await;
        assert!(
            result.is_err(),
            "the colliding insert must fail the transaction"
        );

        // Rollback completeness, checked three ways: the deleted live
        // generation is present again, no retirement record for it exists,
        // and the blocking row is untouched.
        assert_eq!(
            repo.get_session_by_refresh_token(&chain.gen0.refresh_token_hash)
                .await
                .expect("read gen0 after rollback"),
            Some(chain.gen0),
            "the delete must have been rolled back with the rest of the transaction"
        );
        assert_eq!(
            retired_count(&repo).await,
            0,
            "no orphaned retirement record may survive the rollback"
        );
        assert!(
            repo.get_session_by_refresh_token(&blocker.refresh_token_hash)
                .await
                .expect("read blocker after rollback")
                .is_some(),
            "the blocking row must be untouched"
        );
    }

    /// The migration upgrades a `sessions` table that predates the family
    /// columns without touching its rows, is safe to re-run, and legacy rows
    /// read back through the domain type with the sentinel values.
    #[tokio::test]
    async fn session_family_column_migration_upgrades_legacy_table_and_is_idempotent() {
        let options = SqliteConnectOptions::new()
            .filename(":memory:")
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .expect("in-memory pool");

        // Simulate the pre-rotation schema: a sessions table with no family
        // columns, holding one row.
        sqlx::query(
            "CREATE TABLE sessions (
                refresh_token_hash  TEXT PRIMARY KEY,
                user_id             TEXT NOT NULL,
                provider            TEXT NOT NULL,
                expires_at          TEXT NOT NULL,
                device_id           TEXT,
                user_agent          TEXT,
                ip_address          TEXT,
                created_at          TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .expect("create legacy sessions table");
        let now_str = Utc::now().to_rfc3339();
        let expires_str = (Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
        sqlx::query(
            "INSERT INTO sessions (refresh_token_hash, user_id, provider, expires_at, created_at) \
             VALUES ('legacy-hash', 'usr_legacy', 'google', ?1, ?2)",
        )
        .bind(expires_str)
        .bind(now_str)
        .execute(&pool)
        .await
        .expect("insert legacy row");

        // Run the upgrade path twice: every statement must stay idempotent.
        for _ in 0..2 {
            for statement in MIGRATIONS.split(';') {
                let trimmed = statement.trim();
                if !trimmed.is_empty() {
                    sqlx::query(trimmed)
                        .execute(&pool)
                        .await
                        .expect("re-running a migration statement should not error");
                }
            }
            ensure_session_family_columns(&pool)
                .await
                .expect("ensure_session_family_columns");
        }

        let repo = SqliteRepository::new(pool, TEST_REUSE_RETENTION_SECS);
        let legacy = repo
            .get_session_by_refresh_token("legacy-hash")
            .await
            .expect("read migrated legacy row")
            .expect("migrated row must survive");
        assert_eq!(
            legacy.family_id, "",
            "NULL column lands on the empty sentinel"
        );
        assert!(!is_valid_family_id(&legacy.family_id));
        assert_eq!(legacy.generation, 0);
        assert_eq!(legacy.rotated_at, None);

        // And the upgraded table accepts canonical rows.
        let canonical = oidc_exchange_test_utils::session_contract::generation_session(
            "usr_new",
            &oidc_exchange_test_utils::session_contract::fixture_family_id(
                "sqlite-migration:new-row",
            ),
            0,
            "canonical-hash".to_string(),
            Utc::now() + chrono::Duration::hours(1),
            Utc::now(),
            None,
        );
        repo.store_refresh_token(&canonical)
            .await
            .expect("store canonical row post-migration");
        assert_eq!(
            repo.resolve_refresh_token(&canonical.refresh_token_hash)
                .await
                .expect("resolve canonical row"),
            oidc_exchange_core::domain::RefreshResolution::Live(canonical),
            "the upgraded table must accept and classify canonical rows"
        );
    }

    /// Cleanup sweeps expired sessions and expired retirement records alike,
    /// its count covering both tables, and leaves live state alone.
    #[tokio::test]
    async fn cleanup_counts_expired_sessions_and_retired_records_together() {
        let repo = create_test_repo().await;
        use oidc_exchange_test_utils::session_contract::family_chain;

        let chain = family_chain("sqlite:cleanup", 0, "usr_cleanup");
        repo.store_refresh_token(&chain.gen0)
            .await
            .expect("store gen0");
        assert!(
            repo.rotate_refresh_token(&chain.gen0.refresh_token_hash, &chain.gen1)
                .await
                .expect("rotate"),
            "rotation must win"
        );

        // Force-expire everything by rewriting the timestamps in place: the
        // live generation, and the retention deadline of the retirement record.
        let past = (Utc::now() - chrono::Duration::hours(2)).to_rfc3339();
        sqlx::query("UPDATE sessions SET expires_at = ?1 WHERE refresh_token_hash = ?2")
            .bind(past.clone())
            .bind(chain.gen1.refresh_token_hash.clone())
            .execute(&repo.pool)
            .await
            .expect("expire live generation");
        sqlx::query(
            "UPDATE retired_refresh_tokens SET expires_at = ?1 WHERE refresh_token_hash = ?2",
        )
        .bind(past)
        .bind(chain.gen0.refresh_token_hash.clone())
        .execute(&repo.pool)
        .await
        .expect("expire retirement record");

        let removed = repo.cleanup_expired_sessions().await.expect("cleanup");
        assert_eq!(
            removed, 2,
            "cleanup must count one expired session plus one expired retirement record"
        );
        assert_eq!(retired_count(&repo).await, 0);
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

    /// `revoke_all_user_sessions` removes the user's live generations *and*
    /// their retained retirement records, leaving other users untouched.
    #[tokio::test]
    async fn revoke_all_user_sessions_sweeps_retired_records_of_that_user_only() {
        let repo = create_test_repo().await;
        use oidc_exchange_test_utils::session_contract::family_chain;

        let mine = family_chain("sqlite:revoke-all", 0, "usr_mine");
        let theirs = family_chain("sqlite:revoke-all", 1, "usr_theirs");
        repo.store_refresh_token(&mine.gen0)
            .await
            .expect("store mine");
        repo.store_refresh_token(&theirs.gen0)
            .await
            .expect("store theirs");
        assert!(
            repo.rotate_refresh_token(&mine.gen0.refresh_token_hash, &mine.gen1)
                .await
                .expect("rotate mine"),
            "my rotation must win"
        );
        assert!(
            repo.rotate_refresh_token(&theirs.gen0.refresh_token_hash, &theirs.gen1)
                .await
                .expect("rotate theirs"),
            "their rotation must win"
        );
        assert_eq!(retired_count(&repo).await, 2, "one record per rotation");

        repo.revoke_all_user_sessions("usr_mine")
            .await
            .expect("revoke all mine");

        use oidc_exchange_core::domain::RefreshResolution::*;
        assert_eq!(
            repo.resolve_refresh_token(&mine.gen1.refresh_token_hash)
                .await
                .expect("resolve my live"),
            Unknown,
            "my live generation must be gone"
        );
        assert_eq!(
            repo.resolve_refresh_token(&mine.gen0.refresh_token_hash)
                .await
                .expect("resolve my retired"),
            Unknown,
            "my retirement record must be gone"
        );
        match repo
            .resolve_refresh_token(&theirs.gen1.refresh_token_hash)
            .await
            .expect("resolve their live")
        {
            Live(session) => assert_eq!(session.user_id, "usr_theirs"),
            other => panic!("another user's family must survive my revocation, got {other:?}"),
        }
        assert_eq!(
            retired_count(&repo).await,
            1,
            "only the other user's retirement record remains"
        );
    }

    /// A retirement record past its retention deadline classifies as Unknown
    /// even before cleanup physically deletes it — the reuse window has
    /// closed, so re-presentation is refused silently rather than alarmed on.
    #[tokio::test]
    async fn expired_retirement_record_resolves_unknown_before_cleanup() {
        let repo = create_test_repo().await;
        use oidc_exchange_test_utils::session_contract::family_chain;

        let chain = family_chain("sqlite:expired-record", 0, "usr_expired_record");
        repo.store_refresh_token(&chain.gen0)
            .await
            .expect("store gen0");
        assert!(
            repo.rotate_refresh_token(&chain.gen0.refresh_token_hash, &chain.gen1)
                .await
                .expect("rotate"),
            "rotation must win"
        );

        // Expire only the retirement record; the successor stays live.
        let past = (Utc::now() - chrono::Duration::seconds(1)).to_rfc3339();
        sqlx::query(
            "UPDATE retired_refresh_tokens SET expires_at = ?1 WHERE refresh_token_hash = ?2",
        )
        .bind(past)
        .bind(chain.gen0.refresh_token_hash.clone())
        .execute(&repo.pool)
        .await
        .expect("expire retirement record");

        use oidc_exchange_core::domain::RefreshResolution::*;
        assert_eq!(
            repo.resolve_refresh_token(&chain.gen0.refresh_token_hash)
                .await
                .expect("resolve"),
            Unknown,
            "an expired record must classify Unknown, not Superseded"
        );
        // Cleanup removes it and reports it.
        let removed = repo.cleanup_expired_sessions().await.expect("cleanup");
        assert_eq!(removed, 1, "the sweep must remove exactly the dead record");
    }

    // -- Single-use conformance (shared suite in test-utils) --------------------

    use oidc_exchange_test_utils::single_use_conformance as conformance;

    #[tokio::test]
    async fn single_use_first_claim_wins_duplicate_loses() {
        let repo = create_test_repo().await;
        conformance::first_claim_wins_duplicate_loses(&repo).await;
    }

    #[tokio::test]
    async fn single_use_consume_live_record_exactly_once() {
        let repo = create_test_repo().await;
        conformance::consume_live_record_exactly_once(&repo).await;
    }

    #[tokio::test]
    async fn single_use_expired_record_is_absent_to_put_and_take() {
        let repo = create_test_repo().await;
        conformance::expired_record_is_absent_to_put_and_take(&repo).await;
    }

    /// Concurrency needs a multi-connection file-backed pool; `:memory:` would
    /// serialize the racers at the connection level.
    #[tokio::test]
    async fn single_use_concurrent_put_has_exactly_one_winner() {
        let (repo, _dir) = create_racing_test_repo().await;
        conformance::concurrent_put_has_exactly_one_winner(std::sync::Arc::new(repo)).await;
    }

    #[tokio::test]
    async fn single_use_concurrent_take_has_exactly_one_winner() {
        let (repo, _dir) = create_racing_test_repo().await;
        conformance::concurrent_take_has_exactly_one_winner(std::sync::Arc::new(repo)).await;
    }

    #[tokio::test]
    async fn single_use_cleanup_sweeps_expired_records_and_counts_both_kinds() {
        let repo = create_test_repo().await;
        conformance::cleanup_sweeps_expired_single_use_records(&repo).await;
    }

    /// The `single_use` DDL must be idempotent: re-running the migration statements on
    /// a database that already has the table (and rows in it) must not error or lose
    /// data, so startup migrations stay safe to re-run on every boot.
    #[tokio::test]
    async fn single_use_ddl_is_idempotent_across_repeated_migrations() {
        let repo = create_test_repo().await;

        let key = "nonce:ddl_idempotency";
        let claimed = repo
            .put_single_use(key, Utc::now() + chrono::Duration::minutes(5))
            .await
            .expect("claim before migration replay");
        assert!(claimed);

        for _ in 0..2 {
            for statement in MIGRATIONS.split(';') {
                let trimmed = statement.trim();
                if !trimmed.is_empty() {
                    sqlx::query(trimmed)
                        .execute(&repo.pool)
                        .await
                        .expect("re-running a migration statement should not error");
                }
            }
        }

        let consumed = repo.take_single_use(key).await.expect("take after replay");
        assert!(
            consumed,
            "the pre-existing record must survive migration replay"
        );
    }
}
