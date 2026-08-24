use std::collections::HashMap;
use std::future::Future;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use oidc_exchange_core::domain::{
    is_valid_family_id, NewUser, RefreshResolution, RetiredRefreshToken, Session, User, UserPatch,
    UserStatus,
};
use serde_json::Value;
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};
use tracing::instrument;

use oidc_exchange_core::error::{Error, Result};
use oidc_exchange_core::secret::Secret;
use oidc_exchange_core::ports::{SessionRepository, UserRepository};

pub const MIGRATIONS: &str = r#"
CREATE TABLE IF NOT EXISTS users (
    id              TEXT PRIMARY KEY,
    external_id     TEXT NOT NULL,
    provider        TEXT NOT NULL,
    email           TEXT,
    display_name    TEXT,
    metadata        JSONB NOT NULL DEFAULT '{}',
    claims          JSONB NOT NULL DEFAULT '{}',
    status          TEXT NOT NULL DEFAULT 'active',
    version         BIGINT NOT NULL DEFAULT 1,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- `CREATE TABLE IF NOT EXISTS` above only covers fresh databases; a `users` table that
-- predates the `version` column needs this explicit, idempotent step.
ALTER TABLE users ADD COLUMN IF NOT EXISTS version BIGINT NOT NULL DEFAULT 1;

-- `(external_id, provider)` is unique only among live (non-deleted) users: a soft-deleted
-- user must free its identity for re-registration. `CREATE UNIQUE INDEX IF NOT EXISTS`
-- cannot turn a pre-existing full index into a partial one, so a database that predates
-- this migration needs the index dropped and recreated with the `WHERE` predicate. Safe to
-- re-run on every startup since both statements are idempotent (`IF EXISTS` / `IF NOT EXISTS`).
DROP INDEX IF EXISTS idx_users_external_id_provider;
CREATE UNIQUE INDEX IF NOT EXISTS idx_users_external_id_provider ON users (external_id, provider) WHERE status != 'deleted';

CREATE TABLE IF NOT EXISTS sessions (
    refresh_token_hash  TEXT PRIMARY KEY,
    user_id             TEXT NOT NULL REFERENCES users(id),
    family_id           TEXT,
    generation          BIGINT NOT NULL DEFAULT 0,
    provider            TEXT NOT NULL,
    expires_at          TIMESTAMPTZ NOT NULL,
    rotated_at          TIMESTAMPTZ,
    device_id           TEXT,
    user_agent          TEXT,
    ip_address          TEXT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_sessions_user_id ON sessions (user_id);

-- `CREATE TABLE IF NOT EXISTS` above only covers fresh databases; a `sessions` table
-- that predates refresh-token rotation needs these explicit, idempotent steps. Legacy
-- rows keep NULL `family_id`/`rotated_at` and generation 0; the rotation flow deletes
-- such a row on first redemption without writing a retirement record.
ALTER TABLE sessions ADD COLUMN IF NOT EXISTS family_id TEXT;
ALTER TABLE sessions ADD COLUMN IF NOT EXISTS generation BIGINT NOT NULL DEFAULT 0;
ALTER TABLE sessions ADD COLUMN IF NOT EXISTS rotated_at TIMESTAMPTZ;

CREATE TABLE IF NOT EXISTS retired_refresh_tokens (
    refresh_token_hash  TEXT PRIMARY KEY,
    family_id           TEXT NOT NULL,
    user_id             TEXT NOT NULL,
    successor_hash      TEXT NOT NULL,
    retired_at          TIMESTAMPTZ NOT NULL,
    expires_at          TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_retired_family ON retired_refresh_tokens (family_id);
CREATE INDEX IF NOT EXISTS idx_retired_expires_at ON retired_refresh_tokens (expires_at);

-- Single-use records (nonces and assertion-replay markers): a presence-only digest key
-- plus an expiry. The `expires_at` index serves only the cleanup sweep — both claim
-- operations are keyed lookups that evaluate expiry themselves.
CREATE TABLE IF NOT EXISTS single_use (
    key         TEXT PRIMARY KEY,
    expires_at  TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_single_use_expires_at ON single_use (expires_at);
"#;

pub struct PostgresRepository {
    pool: PgPool,
    /// How long a retirement record stays readable after its rotation:
    /// `retired_at + reuse_retention_secs`, capped per record at the family's
    /// absolute `expires_at` by [`RetiredRefreshToken::retention_deadline`].
    /// Resolved from `[token] refresh_reuse_retention` at bootstrap; injected
    /// here because the store, not the caller, stamps every record's deadline.
    reuse_retention_secs: u64,
}

impl PostgresRepository {
    pub fn new(pool: PgPool, reuse_retention_secs: u64) -> Self {
        assert!(
            reuse_retention_secs > 0,
            "reuse retention must be greater than zero"
        );
        Self {
            pool,
            reuse_retention_secs,
        }
    }

    fn store_err(e: impl std::fmt::Display) -> Error {
        Error::StoreError {
            detail: e.to_string(),
        }
    }
}

/// Postgres SQLSTATE for a unique-constraint violation (`unique_violation`), per
/// <https://www.postgresql.org/docs/current/errcodes-appendix.html>.
const PG_UNIQUE_VIOLATION_CODE: &str = "23505";

/// Maximum number of read-modify-write attempts `update_user` makes against its
/// version-conditional `UPDATE … WHERE id = $1 AND version = $2` before giving up: the
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

/// True when `err` is a Postgres unique-constraint violation, decided by the driver's
/// structured SQLSTATE code (not a substring match on the error message) so callers can
/// distinguish "already registered" from any other insert failure.
fn is_unique_violation(err: &sqlx::Error) -> bool {
    err.as_database_error()
        .and_then(|db_err| db_err.code())
        .is_some_and(|code| code == PG_UNIQUE_VIOLATION_CODE)
}

/// Postgres SQLSTATE for a permission-denied DDL statement (`insufficient_privilege`), per
/// <https://www.postgresql.org/docs/current/errcodes-appendix.html>. A migration denied with
/// this code degrades to a warn-and-probe path (see [`create_pool`]) instead of failing
/// startup outright; every other migration failure still fails fast.
const INSUFFICIENT_PRIVILEGE_SQLSTATE: &str = "42501";

/// True when `code` — the structured SQLSTATE pulled off a failed migration's database error —
/// is the Postgres insufficient-privilege code. Kept pure and independent of `sqlx::Error` (a
/// bare `Option<&str>` in, `bool` out) so it is unit-testable without a live restricted-role
/// connection; `create_pool` is the only caller, passing
/// `err.as_database_error().and_then(|e| e.code())`.
fn is_insufficient_privilege_code(code: Option<&str>) -> bool {
    code == Some(INSUFFICIENT_PRIVILEGE_SQLSTATE)
}

/// Builds the Postgres connection pool and, unless `run_migrations` is `false`, executes the
/// adapter's idempotent [`MIGRATIONS`] DDL before returning — mirroring the SQLite adapter's
/// `create_pool`. `MIGRATIONS` is a multi-statement block, which Postgres refuses to accept as
/// a single prepared statement, so migrations run via `sqlx::raw_sql`'s simple-query protocol
/// rather than `sqlx::query`. With `run_migrations = false`, `create_pool` only connects —
/// for locked-down deployments where the app role has no DDL rights and migrations are applied
/// out-of-band.
///
/// A migration failure whose database error carries [`INSUFFICIENT_PRIVILEGE_SQLSTATE`]
/// (`42501`, the role lacks DDL rights) degrades only after a probe proves the pre-provisioned
/// schema provides every invariant established by [`MIGRATIONS`]: the `users` and `sessions`
/// tables, the unique partial `idx_users_external_id_provider` index, and `users.version`.
/// Every missing, malformed, or unreadable probe result returns the *original* migration error.
/// Every other migration error fails fast unchanged.
async fn migration_invariants_hold(pool: &PgPool) -> bool {
    const INVARIANT_PROBE: &str = "SELECT \
        to_regclass('users') IS NOT NULL AS users_exists, \
        to_regclass('sessions') IS NOT NULL AS sessions_exists, \
        EXISTS ( \
            SELECT 1 \
            FROM pg_index index_definition \
            INNER JOIN pg_class index_relation ON index_relation.oid = index_definition.indexrelid \
            WHERE index_relation.relname = 'idx_users_external_id_provider' \
              AND index_definition.indisunique \
              AND index_definition.indpred IS NOT NULL \
        ) AS external_id_provider_index_valid, \
        EXISTS ( \
            SELECT 1 \
            FROM pg_attribute column_definition \
            WHERE column_definition.attrelid = to_regclass('users') \
              AND column_definition.attname = 'version' \
              AND column_definition.attnum > 0 \
              AND NOT column_definition.attisdropped \
        ) AS users_version_exists";

    let Ok(row) = sqlx::query(INVARIANT_PROBE).fetch_one(pool).await else {
        return false;
    };

    let Ok(users_exists) = row.try_get::<bool, _>("users_exists") else {
        return false;
    };
    let Ok(sessions_exists) = row.try_get::<bool, _>("sessions_exists") else {
        return false;
    };
    let Ok(external_id_provider_index_valid) =
        row.try_get::<bool, _>("external_id_provider_index_valid")
    else {
        return false;
    };
    let Ok(users_version_exists) = row.try_get::<bool, _>("users_version_exists") else {
        return false;
    };

    users_exists && sessions_exists && external_id_provider_index_valid && users_version_exists
}

pub async fn create_pool(
    url: &str,
    max_connections: u32,
    run_migrations: bool,
) -> std::result::Result<PgPool, sqlx::Error> {
    let pool = PgPoolOptions::new()
        .max_connections(max_connections)
        .connect(url)
        .await?;

    if run_migrations {
        if let Err(err) = sqlx::raw_sql(MIGRATIONS).execute(&pool).await {
            let code = err.as_database_error().and_then(|db_err| db_err.code());
            if !is_insufficient_privilege_code(code.as_deref()) {
                return Err(err);
            }

            tracing::warn!(
                sqlstate = INSUFFICIENT_PRIVILEGE_SQLSTATE,
                "postgres migration denied (insufficient privilege); probing pre-provisioned \
                 schema invariants"
            );

            if !migration_invariants_hold(&pool).await {
                // Return the original migration error, not a probe-derived one: the denied
                // DDL is why startup is failing, and a failed or inconclusive probe must not
                // mask that.
                return Err(err);
            }

            tracing::warn!(
                "proceeding despite denied migration DDL: required schema invariants hold"
            );
        }
    }

    Ok(pool)
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

fn row_to_user(row: &sqlx::postgres::PgRow) -> Result<User> {
    Ok(User {
        id: row.try_get("id").map_err(PostgresRepository::store_err)?,
        external_id: row
            .try_get("external_id")
            .map_err(PostgresRepository::store_err)?,
        provider: row
            .try_get("provider")
            .map_err(PostgresRepository::store_err)?,
        email: row
            .try_get("email")
            .map_err(PostgresRepository::store_err)?,
        display_name: row
            .try_get("display_name")
            .map_err(PostgresRepository::store_err)?,
        metadata: serde_json::from_value(
            row.try_get::<Value, _>("metadata")
                .map_err(PostgresRepository::store_err)?,
        )
        .map_err(PostgresRepository::store_err)?,
        claims: serde_json::from_value(
            row.try_get::<Value, _>("claims")
                .map_err(PostgresRepository::store_err)?,
        )
        .map_err(PostgresRepository::store_err)?,
        status: str_to_status(
            row.try_get::<&str, _>("status")
                .map_err(PostgresRepository::store_err)?,
        )?,
        version: row
            .try_get::<i64, _>("version")
            .map_err(PostgresRepository::store_err)? as u64,
        created_at: row
            .try_get("created_at")
            .map_err(PostgresRepository::store_err)?,
        updated_at: row
            .try_get("updated_at")
            .map_err(PostgresRepository::store_err)?,
    })
}

fn row_to_session(row: &sqlx::postgres::PgRow) -> Result<Session> {
    // A row written before rotation shipped has a NULL family_id (the columns
    // are added nullable by MIGRATIONS). The domain type requires the field,
    // so the sentinel is an *empty string* — deliberately not a well-formed
    // `fam_` id, so downstream family operations visibly fail rather than
    // silently matching a family that does not exist. The rotation flow
    // deletes such legacy rows on first redemption without writing a
    // retirement record.
    let family_id: Option<String> = row
        .try_get("family_id")
        .map_err(PostgresRepository::store_err)?;
    let rotated_at: Option<DateTime<Utc>> = row
        .try_get("rotated_at")
        .map_err(PostgresRepository::store_err)?;

    Ok(Session {
        user_id: row
            .try_get("user_id")
            .map_err(PostgresRepository::store_err)?,
        refresh_token_hash: Secret::new(
            row.try_get("refresh_token_hash")
                .map_err(PostgresRepository::store_err)?,
        ),
        family_id: family_id.unwrap_or_default(),
        generation: row
            .try_get::<i64, _>("generation")
            .map_err(PostgresRepository::store_err)? as u32,
        provider: row
            .try_get("provider")
            .map_err(PostgresRepository::store_err)?,
        expires_at: row
            .try_get("expires_at")
            .map_err(PostgresRepository::store_err)?,
        rotated_at,
        device_id: row
            .try_get("device_id")
            .map_err(PostgresRepository::store_err)?,
        user_agent: row
            .try_get("user_agent")
            .map_err(PostgresRepository::store_err)?,
        ip_address: row
            .try_get("ip_address")
            .map_err(PostgresRepository::store_err)?,
        created_at: row
            .try_get("created_at")
            .map_err(PostgresRepository::store_err)?,
    })
}

fn row_to_retired(row: &sqlx::postgres::PgRow) -> Result<RetiredRefreshToken> {
    // Round-trip revalidation (store-read boundary): every column of a
    // retirement record is NOT NULL in the DDL, so a missing or mistyped
    // attribute is corruption, not absence, and must surface as a store error.
    Ok(RetiredRefreshToken {
        refresh_token_hash: row
            .try_get("refresh_token_hash")
            .map_err(PostgresRepository::store_err)?,
        family_id: row
            .try_get("family_id")
            .map_err(PostgresRepository::store_err)?,
        user_id: row
            .try_get("user_id")
            .map_err(PostgresRepository::store_err)?,
        successor_hash: row
            .try_get("successor_hash")
            .map_err(PostgresRepository::store_err)?,
        retired_at: row
            .try_get("retired_at")
            .map_err(PostgresRepository::store_err)?,
        expires_at: row
            .try_get("expires_at")
            .map_err(PostgresRepository::store_err)?,
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

/// Insert one session row on an existing transaction or the pool. Shared by
/// `store_refresh_token` and the winning arm of `rotate_refresh_token` so
/// both write paths bind identical columns.
///
/// `replace` selects the conflict behaviour: the store path is idempotent
/// (`ON CONFLICT DO UPDATE`, preserving its existing semantics), while
/// rotation inserts plainly so a replacement hash colliding with any existing
/// live row fails the transaction loudly instead of clobbering it.
async fn insert_session_tx<'e, E>(executor: E, session: &Session, replace: bool) -> Result<()>
where
    E: sqlx::PgExecutor<'e>,
{
    let conflict_clause = if replace {
        "ON CONFLICT (refresh_token_hash) DO UPDATE SET \
         user_id = EXCLUDED.user_id, family_id = EXCLUDED.family_id, \
         generation = EXCLUDED.generation, provider = EXCLUDED.provider, \
         expires_at = EXCLUDED.expires_at, rotated_at = EXCLUDED.rotated_at, \
         device_id = EXCLUDED.device_id, user_agent = EXCLUDED.user_agent, \
         ip_address = EXCLUDED.ip_address, created_at = EXCLUDED.created_at"
    } else {
        ""
    };

    sqlx::query(&format!(
        "INSERT INTO sessions (refresh_token_hash, user_id, family_id, generation, provider, expires_at, rotated_at, device_id, user_agent, ip_address, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) {conflict_clause}"
    ))
    .bind(session.refresh_token_hash.expose())
    .bind(&session.user_id)
    .bind(&session.family_id)
    .bind(session.generation as i64)
    .bind(&session.provider)
    .bind(session.expires_at)
    .bind(session.rotated_at)
    .bind(&session.device_id)
    .bind(&session.user_agent)
    .bind(&session.ip_address)
    .bind(session.created_at)
    .execute(executor)
    .await
    .map_err(PostgresRepository::store_err)?;

    Ok(())
}

#[async_trait]
impl UserRepository for PostgresRepository {
    #[instrument(skip(self), fields(user_id))]
    async fn get_user_by_id(&self, user_id: &str) -> Result<Option<User>> {
        let row = sqlx::query("SELECT * FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(Self::store_err)?;

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
            "SELECT * FROM users WHERE external_id = $1 AND provider = $2 AND status != 'deleted'",
        )
        .bind(external_id)
        .bind(provider)
        .fetch_optional(&self.pool)
        .await
        .map_err(Self::store_err)?;

        match row {
            Some(ref r) => Ok(Some(row_to_user(r)?)),
            None => Ok(None),
        }
    }

    #[instrument(skip(self, user), fields(external_id = %user.external_id, provider = %user.provider))]
    async fn create_user(&self, user: &NewUser) -> Result<User> {
        let now = Utc::now();
        let id = format!("usr_{}", ulid::Ulid::new().to_string().to_lowercase());
        let metadata =
            serde_json::to_value(HashMap::<String, Value>::new()).map_err(Self::store_err)?;
        let claims =
            serde_json::to_value(HashMap::<String, Value>::new()).map_err(Self::store_err)?;
        let status = status_to_str(&UserStatus::Active);

        let row = sqlx::query(
            "INSERT INTO users (id, external_id, provider, email, display_name, metadata, claims, status, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
             RETURNING *",
        )
        .bind(&id)
        .bind(&user.external_id)
        .bind(&user.provider)
        .bind(&user.email)
        .bind(&user.display_name)
        .bind(&metadata)
        .bind(&claims)
        .bind(status)
        .bind(now)
        .bind(now)
        .fetch_one(&self.pool)
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
                Self::store_err(e)
            }
        })?;

        row_to_user(&row)
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

            let metadata_json = serde_json::to_value(&user.metadata).map_err(Self::store_err)?;
            let claims_json = serde_json::to_value(&user.claims).map_err(Self::store_err)?;
            let status_str = status_to_str(&user.status);

            // Version-conditional write: only the writer whose `read_version` still
            // matches the stored row wins, and it increments `version` in the same
            // statement. A concurrent writer that already bumped the row's version makes
            // this affect zero rows (`fetch_optional` returns `None`) instead of silently
            // clobbering the other writer's change.
            let row = sqlx::query(
                "UPDATE users SET email = $1, display_name = $2, metadata = $3, claims = $4, status = $5, updated_at = $6, version = version + 1
                 WHERE id = $7 AND version = $8
                 RETURNING *",
            )
            .bind(&user.email)
            .bind(&user.display_name)
            .bind(&metadata_json)
            .bind(&claims_json)
            .bind(status_str)
            .bind(user.updated_at)
            .bind(user_id)
            .bind(read_version)
            .fetch_optional(&self.pool)
            .await
            .map_err(Self::store_err)?;

            match row {
                Some(row) => Ok(Some(row_to_user(&row)?)),
                None => {
                    tracing::debug!(
                        attempt = attempt_number,
                        max_attempts = UPDATE_MAX_ATTEMPTS,
                        "update_user version conflict, retrying"
                    );
                    Ok(None)
                }
            }
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
            .map_err(Self::store_err)?;

        let mut counts = HashMap::new();
        for row in &rows {
            let status: String = row.try_get("status").map_err(Self::store_err)?;
            let count: i64 = row.try_get("count").map_err(Self::store_err)?;
            counts.insert(status, count as u64);
        }

        Ok(counts)
    }

    #[instrument(skip(self))]
    async fn list_users(&self, offset: u64, limit: u64) -> Result<Vec<User>> {
        let rows = sqlx::query("SELECT * FROM users ORDER BY created_at DESC LIMIT $1 OFFSET $2")
            .bind(limit as i64)
            .bind(offset as i64)
            .fetch_all(&self.pool)
            .await
            .map_err(Self::store_err)?;

        let mut users = Vec::new();
        for row in &rows {
            users.push(row_to_user(row)?);
        }

        Ok(users)
    }
}

#[async_trait]
impl SessionRepository for PostgresRepository {
    #[instrument(skip(self, session), fields(user_id = %session.user_id))]
    async fn store_refresh_token(&self, session: &Session) -> Result<()> {
        insert_session_tx(&self.pool, session, true).await
    }

    // The digest argument is skipped explicitly (not left to a name collision with the
    // schema field) so a parameter rename cannot silently re-expose the lookup key.
    #[instrument(skip(self, token_hash), fields(token_hash))]
    async fn get_session_by_refresh_token(
        &self,
        token_hash: &Secret<String>,
    ) -> Result<Option<Session>> {
        let row = sqlx::query("SELECT * FROM sessions WHERE refresh_token_hash = $1")
            .bind(token_hash.expose())
            .fetch_optional(&self.pool)
            .await
            .map_err(Self::store_err)?;

        match row {
            Some(ref r) => Ok(Some(row_to_session(r)?)),
            None => Ok(None),
        }
    }

    /// Classify `token_hash` against live generations first, then retained
    /// retirement records (SR1). Postgres's default READ COMMITTED isolation
    /// gives every query the latest committed snapshot, so each answer
    /// reflects the most recent write. A record past its retention deadline
    /// answers `Unknown` until the sweep physically deletes it — reuse
    /// detection must not fire on a window that has closed.
    #[instrument(skip(self, token_hash), fields(token_hash))]
    async fn resolve_refresh_token(&self, token_hash: &str) -> Result<RefreshResolution> {
        assert!(
            !token_hash.is_empty(),
            "resolve_refresh_token: token_hash must not be empty"
        );
        if let Some(session) = self.get_session_by_refresh_token(&Secret::new(token_hash.to_string())).await? {
            return Ok(RefreshResolution::Live(session));
        }

        let Some(record) = self.get_retired_record(token_hash).await? else {
            return Ok(RefreshResolution::Unknown);
        };
        if record.expires_at <= Utc::now() {
            return Ok(RefreshResolution::Unknown);
        }

        match self
            .get_session_by_refresh_token(&Secret::new(record.successor_hash.clone()))
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
    /// moved the live generation first (Postgres READ COMMITTED evaluates each
    /// statement against the latest committed snapshot), so the transaction
    /// rolls back having written nothing and this returns `false`.
    ///
    /// A live row carrying the empty-family sentinel is a pre-rotation legacy
    /// row: its first redemption deletes it and installs the replacement
    /// *without* a retirement record — there is no prior generation to detect
    /// reuse against — and the replacement carries whatever family the caller
    /// minted. The store never invents one.
    #[instrument(skip(self, live_hash, replacement), fields(token_hash, user_id = %replacement.user_id))]
    async fn rotate_refresh_token(&self, live_hash: &str, replacement: &Session) -> Result<bool> {
        assert!(
            live_hash != replacement.refresh_token_hash.expose().as_str(),
            "rotate_refresh_token: replacement must be a fresh generation"
        );
        assert!(
            is_valid_family_id(&replacement.family_id),
            "rotate_refresh_token: malformed replacement family id {:?}",
            replacement.family_id
        );

        let mut tx = self.pool.begin().await.map_err(Self::store_err)?;

        // Read the live row inside the transaction: its existence is the CAS
        // condition and its family/user identity decides between the normal
        // three-effect rotation and the legacy first-redemption swap.
        let live_row = sqlx::query("SELECT * FROM sessions WHERE refresh_token_hash = $1")
            .bind(live_hash)
            .fetch_optional(&mut *tx)
            .await
            .map_err(Self::store_err)?;
        let Some(live_row) = live_row else {
            // The condition failed: a concurrent redemption moved the live
            // generation first. Roll back and write nothing.
            tx.rollback().await.map_err(Self::store_err)?;
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

        let deleted = sqlx::query("DELETE FROM sessions WHERE refresh_token_hash = $1")
            .bind(live_hash)
            .execute(&mut *tx)
            .await
            .map_err(Self::store_err)?;
        if deleted.rows_affected() == 0 {
            // The row the SELECT observed was concurrently removed and the
            // delete lost the race; the transaction must still roll back.
            tx.rollback().await.map_err(Self::store_err)?;
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
                 VALUES ($1, $2, $3, $4, $5, $6)",
            )
            .bind(&record.refresh_token_hash)
            .bind(&record.family_id)
            .bind(&record.user_id)
            .bind(&record.successor_hash)
            .bind(record.retired_at)
            .bind(record.expires_at)
            .execute(&mut *tx)
            .await
            .map_err(Self::store_err)?;
        }

        insert_session_tx(&mut *tx, replacement, false).await?;

        tx.commit().await.map_err(Self::store_err)?;
        Ok(true)
    }

    #[instrument(skip(self, token_hash), fields(token_hash))]
    async fn revoke_session(&self, token_hash: &Secret<String>) -> Result<()> {
        let token_hash = token_hash.expose().as_str();
        sqlx::query("DELETE FROM sessions WHERE refresh_token_hash = $1")
            .bind(token_hash)
            .execute(&self.pool)
            .await
            .map_err(Self::store_err)?;

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

        let mut tx = self.pool.begin().await.map_err(Self::store_err)?;
        let live = sqlx::query("DELETE FROM sessions WHERE family_id = $1")
            .bind(family_id)
            .execute(&mut *tx)
            .await
            .map_err(Self::store_err)?;
        let retired = sqlx::query("DELETE FROM retired_refresh_tokens WHERE family_id = $1")
            .bind(family_id)
            .execute(&mut *tx)
            .await
            .map_err(Self::store_err)?;
        tx.commit().await.map_err(Self::store_err)?;

        Ok(live.rows_affected() + retired.rows_affected())
    }

    #[instrument(skip(self))]
    async fn count_active_sessions(&self) -> Result<u64> {
        let row = sqlx::query("SELECT COUNT(*) as count FROM sessions WHERE expires_at > NOW()")
            .fetch_one(&self.pool)
            .await
            .map_err(Self::store_err)?;

        let count: i64 = row.try_get("count").map_err(Self::store_err)?;
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
        let mut tx = self.pool.begin().await.map_err(Self::store_err)?;
        sqlx::query("DELETE FROM sessions WHERE user_id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await
            .map_err(Self::store_err)?;
        sqlx::query("DELETE FROM retired_refresh_tokens WHERE user_id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await
            .map_err(Self::store_err)?;
        tx.commit().await.map_err(Self::store_err)?;

        Ok(())
    }

    #[instrument(skip(self))]
    async fn cleanup_expired_sessions(&self) -> Result<u64> {
        // The sweep covers sessions, retirement records, and expired
        // single-use records alike, and its count is the combined number
        // deleted (the port contract).
        let mut tx = self.pool.begin().await.map_err(Self::store_err)?;
        let sessions = sqlx::query("DELETE FROM sessions WHERE expires_at < NOW()")
            .execute(&mut *tx)
            .await
            .map_err(Self::store_err)?;
        let retired = sqlx::query("DELETE FROM retired_refresh_tokens WHERE expires_at < NOW()")
            .execute(&mut *tx)
            .await
            .map_err(Self::store_err)?;
        let single_use = sqlx::query("DELETE FROM single_use WHERE expires_at < NOW()")
            .execute(&mut *tx)
            .await
            .map_err(Self::store_err)?;
        tx.commit().await.map_err(Self::store_err)?;

        Ok(sessions.rows_affected() + retired.rows_affected() + single_use.rows_affected())
    }

    #[instrument(skip(self, key))]
    async fn put_single_use(&self, key: &str, expires_at: DateTime<Utc>) -> Result<bool> {
        assert!(!key.is_empty(), "single-use key must be non-empty");

        // Insert-if-absent, with a live conflicting row overwritten only when it has
        // already expired (`WHERE single_use.expires_at < now()`): rows_affected is 1
        // for exactly the winning claim, 0 when a live record holds the key. The
        // primary key makes check-and-insert one atomic statement.
        let result = sqlx::query(
            "INSERT INTO single_use (key, expires_at) VALUES ($1, $2) \
             ON CONFLICT (key) DO UPDATE SET expires_at = EXCLUDED.expires_at \
             WHERE single_use.expires_at < $3",
        )
        .bind(key)
        .bind(expires_at)
        .bind(Utc::now())
        .execute(&self.pool)
        .await
        .map_err(Self::store_err)?;

        Ok(result.rows_affected() == 1)
    }

    #[instrument(skip(self, key))]
    async fn take_single_use(&self, key: &str) -> Result<bool> {
        assert!(!key.is_empty(), "single-use key must be non-empty");

        // Remove-and-report in one statement: only a live record (expiry still ahead of
        // now) matches, so an absent, burned, or expired key deletes zero rows.
        let row =
            sqlx::query("DELETE FROM single_use WHERE key = $1 AND expires_at > NOW() RETURNING 1")
                .bind(key)
                .fetch_optional(&self.pool)
                .await
                .map_err(Self::store_err)?;

        Ok(row.is_some())
    }
}

impl PostgresRepository {
    /// Fetch one retirement record by hash, if it is still retained. Inherent
    /// helper (not a port method): `resolve_refresh_token` needs the raw
    /// record to evaluate the successor pointer, while `/revoke`'s liveness
    /// lookup deliberately does not see retirement records as sessions.
    async fn get_retired_record(&self, token_hash: &str) -> Result<Option<RetiredRefreshToken>> {
        let row = sqlx::query("SELECT * FROM retired_refresh_tokens WHERE refresh_token_hash = $1")
            .bind(token_hash)
            .fetch_optional(&self.pool)
            .await
            .map_err(Self::store_err)?;

        match row {
            Some(ref r) => Ok(Some(row_to_retired(r)?)),
            None => Ok(None),
        }
    }
}

// ---------------------------------------------------------------------------
// Integration tests (require a live PostgreSQL instance)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use oidc_exchange_core::domain::INITIAL_USER_VERSION;

    /// Reuse-retention window used by every test repository: one hour — short
    /// enough that deadline arithmetic stays inside a test's lifetime, and
    /// positive per the constructor's precondition.
    const TEST_REUSE_RETENTION_SECS: u64 = 3600;

    /// `postgres://…` URL for a scratch database, overridable via
    /// `POSTGRES_TEST_URL`. Start one with:
    /// `docker run -e POSTGRES_PASSWORD=postgres -p 5432:5432 postgres:16`.
    fn test_database_url() -> String {
        std::env::var("POSTGRES_TEST_URL")
            .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/postgres".to_string())
    }

    /// Session-scoped Postgres advisory lock key used to serialize the schema
    /// reset (`DROP`/`CREATE TABLE`) in [`create_test_repo`]. `cargo nextest`
    /// runs `#[ignore]`d tests as separate OS processes, and both Postgres
    /// tests below share the one scratch database, so without this lock their
    /// concurrent `DROP TABLE` / `CREATE TABLE IF NOT EXISTS` calls can race
    /// (observed as duplicate-key errors on `pg_type`). The value is
    /// arbitrary; it only needs to be a stable constant shared by every
    /// caller of `create_test_repo`.
    const TEST_SCHEMA_ADVISORY_LOCK_KEY: i64 = 8_675_309;

    async fn create_test_repo() -> PostgresRepository {
        // Migrations are run explicitly below (after the schema reset), so connect only.
        let pool = create_pool(&test_database_url(), 5, false)
            .await
            .expect("failed to connect to test Postgres instance");

        // Advisory locks are session-scoped, so the lock/unlock pair and the
        // schema reset in between must all run on the same connection rather
        // than through the pool (which could hand different queries to
        // different connections).
        let mut conn = pool
            .acquire()
            .await
            .expect("acquire dedicated schema-setup connection");
        sqlx::query("SELECT pg_advisory_lock($1)")
            .bind(TEST_SCHEMA_ADVISORY_LOCK_KEY)
            .execute(&mut *conn)
            .await
            .expect("acquire test schema lock");

        // Fresh scratch schema per run so the unique index / version DEFAULT can be
        // exercised without state left over from a previous run.
        sqlx::query("DROP TABLE IF EXISTS sessions")
            .execute(&mut *conn)
            .await
            .expect("drop sessions");
        sqlx::query("DROP TABLE IF EXISTS retired_refresh_tokens")
            .execute(&mut *conn)
            .await
            .expect("drop retired_refresh_tokens");
        sqlx::query("DROP TABLE IF EXISTS users")
            .execute(&mut *conn)
            .await
            .expect("drop users");
        // `MIGRATIONS` contains multiple statements, which Postgres refuses to
        // accept as a single prepared statement (`sqlx::query`); `raw_sql` sends
        // it unprepared instead, the way a migration runner would.
        sqlx::raw_sql(MIGRATIONS)
            .execute(&mut *conn)
            .await
            .expect("run migrations");

        sqlx::query("SELECT pg_advisory_unlock($1)")
            .bind(TEST_SCHEMA_ADVISORY_LOCK_KEY)
            .execute(&mut *conn)
            .await
            .expect("release test schema lock");
        drop(conn);

        PostgresRepository::new(pool, TEST_REUSE_RETENTION_SECS)
    }

    /// A [`PostgresRepository`] backed by a fresh, private Postgres schema named
    /// `schema_name` (selected via `search_path`), so this test's rows and DDL cannot
    /// race or corrupt the shared `public` scratch schema — or each other — the way two
    /// concurrent [`create_test_repo`] resets can (`cargo nextest` runs `#[ignore]`d
    /// tests as separate, potentially concurrent OS processes). `create_test_repo`'s
    /// advisory lock only serializes its own reset window, not a whole test's lifetime,
    /// so any test doing more than a quick read/write against the shared schema — most
    /// of all, one that drops a table — needs this instead. Every caller below passes a
    /// distinct `schema_name` so the tests below cannot race one another either.
    async fn create_isolated_schema_repo(schema_name: &str) -> PostgresRepository {
        // Only used to drop/create the isolated schema against the default search_path;
        // migrations run afterwards against that schema via `search_path`, so skip them here.
        let bootstrap_pool = create_pool(&test_database_url(), 1, false)
            .await
            .expect("failed to connect to test Postgres instance for schema bootstrap");
        sqlx::query(&format!("DROP SCHEMA IF EXISTS {schema_name} CASCADE"))
            .execute(&bootstrap_pool)
            .await
            .expect("drop isolated test schema");
        sqlx::query(&format!("CREATE SCHEMA {schema_name}"))
            .execute(&bootstrap_pool)
            .await
            .expect("create isolated test schema");
        bootstrap_pool.close().await;

        let options: sqlx::postgres::PgConnectOptions = test_database_url()
            .parse()
            .expect("parse test database url");
        let options = options.options([("search_path", schema_name)]);

        let pool = PgPoolOptions::new()
            .max_connections(2)
            .connect_with(options)
            .await
            .expect("connect with isolated schema search_path");

        sqlx::raw_sql(MIGRATIONS)
            .execute(&pool)
            .await
            .expect("run migrations in isolated schema");

        PostgresRepository::new(pool, TEST_REUSE_RETENTION_SECS)
    }

    #[tokio::test]
    #[ignore] // Requires a live Postgres: see `test_database_url`.
    async fn create_user_round_trips_initial_version() {
        let repo = create_test_repo().await;

        let created = repo
            .create_user(&NewUser {
                external_id: "google|pg_version_test".to_string(),
                provider: "google".to_string(),
                email: Some("pg@example.com".to_string()),
                display_name: None,
            })
            .await
            .expect("create_user");
        assert_eq!(created.version, INITIAL_USER_VERSION);

        let fetched = repo
            .get_user_by_id(&created.id)
            .await
            .expect("get_user_by_id")
            .expect("user should exist");
        assert_eq!(fetched.version, created.version);
    }

    #[tokio::test]
    #[ignore] // Requires a live Postgres: see `test_database_url`.
    async fn legacy_row_without_version_column_defaults_to_initial_version() {
        let repo = create_test_repo().await;

        // Simulate a pre-migration row: drop the column, then apply the same
        // idempotent `ALTER TABLE … ADD COLUMN IF NOT EXISTS` migrations run.
        sqlx::query("ALTER TABLE users DROP COLUMN version")
            .execute(&repo.pool)
            .await
            .expect("drop version column to simulate a pre-migration row");

        let now = Utc::now();
        sqlx::query(
            "INSERT INTO users (id, external_id, provider, status, created_at, updated_at) \
             VALUES ($1, $2, $3, 'active', $4, $4)",
        )
        .bind("usr_legacy")
        .bind("legacy-ext")
        .bind("google")
        .bind(now)
        .execute(&repo.pool)
        .await
        .expect("insert legacy row");

        sqlx::raw_sql(MIGRATIONS)
            .execute(&repo.pool)
            .await
            .expect("re-run migrations to backfill the version column");

        let user = repo
            .get_user_by_id("usr_legacy")
            .await
            .expect("get_user_by_id")
            .expect("legacy user should be found");
        assert_eq!(user.version, INITIAL_USER_VERSION);
        assert_eq!(user.external_id, "legacy-ext");
    }

    #[tokio::test]
    #[ignore] // Requires a live Postgres: see `test_database_url`.
    async fn create_user_duplicate_external_id_returns_conflict() {
        let repo = create_isolated_schema_repo("oidc_adapter_test_duplicate_conflict").await;

        let new_user = NewUser {
            external_id: "google|pg_dup_test".to_string(),
            provider: "google".to_string(),
            email: Some("pg_dup@example.com".to_string()),
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
    #[ignore] // Requires a live Postgres: see `test_database_url`.
    async fn delete_user_frees_external_id_for_recreation() {
        let repo = create_isolated_schema_repo("oidc_adapter_test_delete_frees_identity").await;

        let new_user = NewUser {
            external_id: "google|pg_delete_frees_test".to_string(),
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
    #[ignore] // Requires a live Postgres: see `test_database_url`.
    async fn partial_unique_index_migration_upgrades_legacy_full_index_and_is_idempotent() {
        let repo = create_isolated_schema_repo("oidc_adapter_test_partial_index_migration").await;

        // Simulate a legacy database: replace the partial index the isolated-schema
        // bootstrap just created with the old full unique index that predates this
        // migration (uniqueness enforced across *all* rows, deleted or not).
        sqlx::query("DROP INDEX IF EXISTS idx_users_external_id_provider")
            .execute(&repo.pool)
            .await
            .expect("drop partial index to simulate a legacy schema");
        sqlx::query(
            "CREATE UNIQUE INDEX idx_users_external_id_provider ON users (external_id, provider)",
        )
        .execute(&repo.pool)
        .await
        .expect("recreate legacy full unique index");

        // Re-running the migration DDL must drop the legacy full index and recreate it as
        // partial — and running it a second time must not error (idempotent).
        sqlx::raw_sql(MIGRATIONS)
            .execute(&repo.pool)
            .await
            .expect("first re-run of migrations should upgrade the legacy index");
        sqlx::raw_sql(MIGRATIONS)
            .execute(&repo.pool)
            .await
            .expect("second re-run of migrations should be a no-op");

        let new_user = NewUser {
            external_id: "google|pg_migration_test".to_string(),
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
    /// still map to `Error::StoreError`, not be misclassified as `Conflict`. Runs against
    /// [`create_isolated_schema_repo`] rather than the shared `public` scratch schema,
    /// since it drops the `users` table entirely (not just rows) to force a non-unique
    /// failure, which would otherwise break any other test process concurrently using
    /// `public.users`.
    #[tokio::test]
    #[ignore] // Requires a live Postgres: see `test_database_url`.
    async fn create_user_non_unique_failure_maps_to_store_error() {
        let repo = create_isolated_schema_repo("oidc_adapter_test_store_error_isolated").await;

        // Drop the tables out from under `create_user` so the insert fails for a reason
        // other than a unique-constraint violation (`undefined_table`, SQLSTATE `42P01`).
        sqlx::query("DROP TABLE IF EXISTS sessions")
            .execute(&repo.pool)
            .await
            .expect("drop sessions table");
        sqlx::query("DROP TABLE IF EXISTS users")
            .execute(&repo.pool)
            .await
            .expect("drop users table");

        let err = repo
            .create_user(&NewUser {
                external_id: "google|pg_store_error_test".to_string(),
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

    /// [`is_unique_violation`] must decide from the driver's structured SQLSTATE code, not
    /// a substring of the error message: a genuine unique-constraint violation reads `true`
    /// off code `23505`, and a differently-coded failure (here, a `NOT NULL` violation)
    /// reads `false`.
    #[tokio::test]
    #[ignore] // Requires a live Postgres: see `test_database_url`.
    async fn is_unique_violation_reads_structured_code_not_message() {
        let repo = create_isolated_schema_repo("oidc_adapter_test_unique_violation_code").await;

        let now = Utc::now();
        sqlx::query(
            "INSERT INTO users (id, external_id, provider, status, created_at, updated_at) \
             VALUES ($1, $2, $3, 'active', $4, $4)",
        )
        .bind("usr_pg_unique_probe")
        .bind("google|pg_unique_probe")
        .bind("google")
        .bind(now)
        .execute(&repo.pool)
        .await
        .expect("seed row for unique-violation probe");

        let unique_violation_err = sqlx::query(
            "INSERT INTO users (id, external_id, provider, status, created_at, updated_at) \
             VALUES ($1, $2, $3, 'active', $4, $4)",
        )
        .bind("usr_pg_unique_probe_2")
        .bind("google|pg_unique_probe")
        .bind("google")
        .bind(now)
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
        assert_eq!(code, PG_UNIQUE_VIOLATION_CODE);

        let not_null_violation_err = sqlx::query(
            "INSERT INTO users (id, external_id, provider, status, created_at, updated_at) \
             VALUES ($1, NULL, $2, 'active', $3, $3)",
        )
        .bind("usr_pg_not_null_probe")
        .bind("google")
        .bind(now)
        .execute(&repo.pool)
        .await
        .expect_err("NULL into a NOT NULL column should fail");
        assert!(
            !is_unique_violation(&not_null_violation_err),
            "a NOT NULL violation must not be misclassified as a unique violation"
        );
    }

    #[tokio::test]
    #[ignore] // Requires a live Postgres: see `test_database_url`.
    async fn racing_suspend_and_claims_patch_ends_suspended() {
        let repo = create_isolated_schema_repo("oidc_adapter_test_version_race").await;

        let created = repo
            .create_user(&NewUser {
                external_id: "google|pg_version_race_test".to_string(),
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

    // -----------------------------------------------------------------
    // Denied-DDL degrade classification
    // -----------------------------------------------------------------

    /// The migration-invariant probe must accept precisely the schema that `MIGRATIONS`
    /// guarantees: both tables, a unique partial identity index, and the optimistic-locking
    /// `version` column. This test requires a local Postgres service via `DATABASE_URL`.
    #[tokio::test]
    async fn migration_invariant_probe_rejects_incomplete_or_wrong_indexes() {
        let Ok(base_url) = std::env::var("DATABASE_URL") else {
            eprintln!(
                "skipping migration_invariant_probe_rejects_incomplete_or_wrong_indexes: \
                 DATABASE_URL is not set"
            );
            return;
        };

        let schema = "oidc_adapter_test_migration_invariants";
        reset_schema(&base_url, schema).await;
        let url = url_with_search_path(&base_url, schema);
        let pool = create_pool(&url, 1, true)
            .await
            .expect("migrate isolated schema");

        assert!(
            migration_invariants_hold(&pool).await,
            "the complete migration schema must satisfy the probe"
        );

        sqlx::query("DROP TABLE sessions")
            .execute(&pool)
            .await
            .expect("drop sessions table");
        assert!(
            !migration_invariants_hold(&pool).await,
            "a missing sessions table must fail the probe"
        );
        sqlx::raw_sql(MIGRATIONS)
            .execute(&pool)
            .await
            .expect("restore complete migration schema");

        sqlx::query("DROP TABLE users CASCADE")
            .execute(&pool)
            .await
            .expect("drop users table");
        assert!(
            !migration_invariants_hold(&pool).await,
            "a missing users table must fail the probe"
        );
        sqlx::raw_sql(MIGRATIONS)
            .execute(&pool)
            .await
            .expect("restore complete migration schema");

        sqlx::query("DROP INDEX idx_users_external_id_provider")
            .execute(&pool)
            .await
            .expect("drop partial unique index");
        assert!(
            !migration_invariants_hold(&pool).await,
            "a missing identity index must fail the probe"
        );

        sqlx::query("CREATE INDEX idx_users_external_id_provider ON users (external_id, provider)")
            .execute(&pool)
            .await
            .expect("create non-unique full index");
        assert!(
            !migration_invariants_hold(&pool).await,
            "a non-unique full identity index must fail the probe"
        );

        sqlx::query("DROP INDEX idx_users_external_id_provider")
            .execute(&pool)
            .await
            .expect("drop non-unique full index");
        sqlx::query(
            "CREATE UNIQUE INDEX idx_users_external_id_provider \
             ON users (external_id, provider)",
        )
        .execute(&pool)
        .await
        .expect("create unique full index");
        assert!(
            !migration_invariants_hold(&pool).await,
            "a unique but non-partial identity index must fail the probe"
        );

        sqlx::query("DROP INDEX idx_users_external_id_provider")
            .execute(&pool)
            .await
            .expect("drop unique full index");
        sqlx::query(
            "CREATE UNIQUE INDEX idx_users_external_id_provider \
             ON users (external_id, provider) WHERE status != 'deleted'",
        )
        .execute(&pool)
        .await
        .expect("restore unique partial index");
        sqlx::query("ALTER TABLE users DROP COLUMN version")
            .execute(&pool)
            .await
            .expect("drop version column");
        assert!(
            !migration_invariants_hold(&pool).await,
            "a schema without users.version must fail the probe"
        );
    }

    /// [`is_insufficient_privilege_code`] must route on the exact SQLSTATE, not "some error
    /// occurred": the insufficient-privilege code (`42501`) reads `true` (degrade path), a
    /// differently-coded failure (here, `undefined_table`, `42P01`) and the no-code case both
    /// read `false` (fail-fast) — proving the branch is genuinely conditional on the code
    /// rather than firing on every migration error.
    #[test]
    fn is_insufficient_privilege_code_routes_only_42501_to_degrade() {
        assert!(
            is_insufficient_privilege_code(Some(INSUFFICIENT_PRIVILEGE_SQLSTATE)),
            "the insufficient-privilege SQLSTATE must route to the degrade branch"
        );
        assert!(
            !is_insufficient_privilege_code(Some("42P01")),
            "a differently-coded database error (undefined_table) must fail fast, not degrade"
        );
        assert!(
            !is_insufficient_privilege_code(None),
            "a database error carrying no code at all must fail fast, not degrade"
        );
    }

    // -----------------------------------------------------------------
    // `create_pool` migrate-on-startup behaviour
    // -----------------------------------------------------------------

    /// Appends a `search_path` connection option to `base_url` so a pool built from the
    /// returned URL operates entirely inside `schema_name`, isolated from `public` and from
    /// every other test's schema. Uses `?options=-c search_path=...` (URL-encoded), the query
    /// form `sqlx::postgres::PgConnectOptions` parses into the same libpq startup option that
    /// [`create_isolated_schema_repo`] sets programmatically via `.options(...)`.
    fn url_with_search_path(base_url: &str, schema_name: &str) -> String {
        let separator = if base_url.contains('?') { '&' } else { '?' };
        format!("{base_url}{separator}options=-c%20search_path%3D{schema_name}")
    }

    /// Drops and recreates `schema_name` against `base_url` (default search_path), so each
    /// caller below starts from a guaranteed-empty schema regardless of what a previous test
    /// run left behind. Connects with `run_migrations = false` since this only manages the
    /// schema itself, not its tables.
    async fn reset_schema(base_url: &str, schema_name: &str) {
        let pool = create_pool(base_url, 1, false)
            .await
            .expect("failed to connect to DATABASE_URL for schema bootstrap");
        sqlx::query(&format!("DROP SCHEMA IF EXISTS {schema_name} CASCADE"))
            .execute(&pool)
            .await
            .expect("drop test schema");
        sqlx::query(&format!("CREATE SCHEMA {schema_name}"))
            .execute(&pool)
            .await
            .expect("create test schema");
        pool.close().await;
    }

    /// Gated on `DATABASE_URL` (skips cleanly, rather than failing, when unset, so
    /// `cargo nextest run --workspace` stays green without a live database configured).
    /// Proves both halves of the migrate-on-startup contract in one run against a real
    /// Postgres instance: `create_pool(url, n, true)` alone — no explicit `raw_sql(MIGRATIONS)`
    /// call, unlike every other test in this module — leaves a fresh schema able to serve
    /// `create_user`/`get_user_by_id`; `create_pool(url, n, false)` against a separate fresh
    /// schema is negative-space coverage that the migration is genuinely conditional, not
    /// always-on — it must leave the `users`/`sessions` tables absent.
    #[tokio::test]
    async fn create_pool_migrates_on_startup_and_run_migrations_false_stays_bare() {
        let Ok(base_url) = std::env::var("DATABASE_URL") else {
            eprintln!(
                "skipping create_pool_migrates_on_startup_and_run_migrations_false_stays_bare: \
                 DATABASE_URL is not set"
            );
            return;
        };

        // `run_migrations = true`, alone, must leave a working schema.
        let migrated_schema = "oidc_adapter_test_migrate_on_startup_true";
        reset_schema(&base_url, migrated_schema).await;
        let migrated_url = url_with_search_path(&base_url, migrated_schema);
        let pool = create_pool(&migrated_url, 2, true)
            .await
            .expect("create_pool(url, n, true) should connect and migrate a fresh schema");
        let repo = PostgresRepository::new(pool, TEST_REUSE_RETENTION_SECS);

        let created = repo
            .create_user(&NewUser {
                external_id: "google|pg_migrate_on_startup_test".to_string(),
                provider: "google".to_string(),
                email: Some("migrate_on_startup@example.com".to_string()),
                display_name: None,
            })
            .await
            .expect("create_user should succeed once create_pool alone has migrated the schema");
        let fetched = repo
            .get_user_by_id(&created.id)
            .await
            .expect("get_user_by_id")
            .expect("the created user must round-trip");
        assert_eq!(fetched.id, created.id, "round-tripped user id must match");
        assert_eq!(
            fetched.external_id, created.external_id,
            "round-tripped user external_id must match"
        );

        // `run_migrations = false`, against a separate fresh schema, must leave it bare.
        let bare_schema = "oidc_adapter_test_migrate_on_startup_false";
        reset_schema(&base_url, bare_schema).await;
        let bare_url = url_with_search_path(&base_url, bare_schema);
        let bare_pool = create_pool(&bare_url, 2, false)
            .await
            .expect("create_pool(url, n, false) should still connect without migrating");

        let row = sqlx::query(
            "SELECT to_regclass('users')::text AS users_reg, \
                    to_regclass('sessions')::text AS sessions_reg",
        )
        .fetch_one(&bare_pool)
        .await
        .expect("probing for the tables should succeed even though neither exists");
        let users_reg: Option<String> = row.try_get("users_reg").expect("users_reg column");
        let sessions_reg: Option<String> =
            row.try_get("sessions_reg").expect("sessions_reg column");
        assert!(
            users_reg.is_none(),
            "run_migrations = false must not create the users table"
        );
        assert!(
            sessions_reg.is_none(),
            "run_migrations = false must not create the sessions table"
        );
    }

    // -----------------------------------------------------------------------
    // Refresh-token rotation and reuse detection (task 03)
    // -----------------------------------------------------------------------

    use oidc_exchange_test_utils::session_contract;

    /// The `sessions.user_id` foreign key requires the fixture users the
    /// shared suite writes sessions for to exist first. Seeds them directly
    /// (their ids are fixed by the harness, while `create_user` mints its
    /// own), relying on the DDL defaults for status/metadata/claims.
    async fn seed_fixture_users(repo: &PostgresRepository, user_ids: &[&str]) {
        for user_id in user_ids {
            let inserted = sqlx::query(
                "INSERT INTO users (id, external_id, provider) VALUES ($1, $2, 'conformance') \
                 ON CONFLICT (id) DO NOTHING",
            )
            .bind(user_id)
            .bind(format!("external|{user_id}"))
            .execute(&repo.pool)
            .await
            .expect("seed fixture user");
            assert!(
                inserted.rows_affected() <= 1,
                "seeding {user_id} may not insert more than one row"
            );
        }
    }

    /// Insert a pre-rotation (legacy) session row exactly the way the
    /// pre-migration schema would have written it: NULL `family_id`,
    /// generation 0, NULL `rotated_at`.
    async fn seed_legacy_session(repo: &PostgresRepository, token_hash: &str, user_id: &str) {
        seed_fixture_users(repo, &[user_id]).await;
        let now = Utc::now();
        let expires_at = now + chrono::Duration::hours(24);
        sqlx::query(
            "INSERT INTO sessions (refresh_token_hash, user_id, family_id, generation, provider, expires_at, rotated_at, device_id, user_agent, ip_address, created_at) \
             VALUES ($1, $2, NULL, 0, 'google', $3, NULL, NULL, NULL, NULL, $4)",
        )
        .bind(token_hash)
        .bind(user_id)
        .bind(expires_at)
        .bind(now)
        .execute(&repo.pool)
        .await
        .expect("seed legacy session row");
    }

    async fn retired_count(repo: &PostgresRepository) -> i64 {
        let row = sqlx::query("SELECT COUNT(*) AS count FROM retired_refresh_tokens")
            .fetch_one(&repo.pool)
            .await
            .expect("count retirement records");
        row.try_get::<i64, _>("count").expect("count column")
    }

    /// The full SR1–SR5 shared suite against the Postgres store. One tag
    /// namespaces every fixture the suite creates; the isolated schema keeps
    /// concurrent ignored-test processes from touching each other's rows.
    #[tokio::test]
    #[ignore] // Requires a live Postgres: see `test_database_url`.
    async fn postgres_session_store_meets_sr1_through_sr5() {
        let repo = create_isolated_schema_repo("oidc_pg_session_conformance").await;
        seed_fixture_users(&repo, &["usr_conformance", "usr_shared"]).await;
        session_contract::assert_full_conformance(&repo, "postgres-session-conformance").await;
    }

    /// A legacy row's first redemption swaps atomically but writes no
    /// retirement record — there is no prior generation to detect reuse
    /// against — and the presented hash reads Unknown afterwards. The
    /// replacement carries the caller's newly-minted family; nothing here
    /// synthesizes one.
    #[tokio::test]
    #[ignore] // Requires a live Postgres: see `test_database_url`.
    async fn legacy_row_first_redemption_swaps_without_retirement_record() {
        let repo = create_isolated_schema_repo("oidc_pg_legacy_first_redemption").await;
        let legacy_hash = session_contract::fixture_hash("postgres-legacy:first-redemption");
        seed_legacy_session(&repo, &legacy_hash, "usr_legacy").await;

        // Classification is storage-factual: the sentinel-carrying row is Live.
        let live = repo
            .resolve_refresh_token(&legacy_hash)
            .await
            .expect("resolve legacy row");
        match &live {
            RefreshResolution::Live(session) => {
                assert_eq!(session.family_id, "", "sentinel family on read");
                assert_eq!(session.generation, 0);
                assert_eq!(session.rotated_at, None);
            }
            other => panic!("the stored legacy row must resolve Live, got {other:?}"),
        }

        let base = Utc::now();
        let new_family = session_contract::fixture_family_id("postgres-legacy:new-fam");
        assert!(is_valid_family_id(&new_family));
        if let RefreshResolution::Live(legacy) = &live {
            let replacement = Session {
                refresh_token_hash: Secret::new(format!("{legacy_hash}-next")),
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
            assert_eq!(
                retired_count(&repo).await,
                0,
                "a legacy first redemption must not leave a retirement record"
            );
        } else {
            unreachable!("checked Live above");
        }
    }

    /// Transaction rollback: when the replacement insert fails mid-transaction
    /// (its hash colliding with an existing live row), the whole unit rolls
    /// back — the live generation the delete removed comes back, and no
    /// orphaned retirement record is left behind (SR2's all-or-nothing rule
    /// under a real failure, not just a lost CAS).
    #[tokio::test]
    #[ignore] // Requires a live Postgres: see `test_database_url`.
    async fn failed_replacement_insert_rolls_back_delete_and_retirement() {
        let repo = create_isolated_schema_repo("oidc_pg_rollback").await;
        seed_fixture_users(&repo, &["usr_rollback", "usr_blocker"]).await;
        let chain = session_contract::family_chain("postgres:rollback", 0, "usr_rollback");
        repo.store_refresh_token(&chain.gen0)
            .await
            .expect("store gen0");

        // An unrelated live row already occupying the replacement's hash: the
        // mid-transaction collision that forces the rollback.
        let blocker = Session {
            refresh_token_hash: chain.gen1.refresh_token_hash.clone(),
            family_id: session_contract::fixture_family_id("postgres:rollback:blocker-fam"),
            user_id: "usr_blocker".to_string(),
            ..chain.gen1.clone()
        };
        repo.store_refresh_token(&blocker)
            .await
            .expect("store blocker");

        let result = repo
            .rotate_refresh_token(chain.gen0.refresh_token_hash.expose(), &chain.gen1)
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

    /// Cleanup sweeps expired sessions and expired retirement records alike,
    /// its count covering both tables. Runs in an isolated schema so the
    /// exact global counts cannot be disturbed by a concurrently running
    /// ignored test.
    #[tokio::test]
    #[ignore] // Requires a live Postgres: see `test_database_url`.
    async fn cleanup_counts_expired_sessions_and_retired_records_together() {
        let repo = create_isolated_schema_repo("oidc_pg_cleanup").await;
        seed_fixture_users(&repo, &["usr_cleanup"]).await;
        let chain = session_contract::family_chain("postgres:cleanup", 0, "usr_cleanup");
        repo.store_refresh_token(&chain.gen0)
            .await
            .expect("store gen0");
        assert!(
            repo.rotate_refresh_token(chain.gen0.refresh_token_hash.expose(), &chain.gen1)
                .await
                .expect("rotate"),
            "rotation must win"
        );

        // Force-expire everything by rewriting the timestamps in place: the
        // live generation, and the retention deadline of the retirement record.
        let past = Utc::now() - chrono::Duration::hours(2);
        sqlx::query("UPDATE sessions SET expires_at = $1 WHERE refresh_token_hash = $2")
            .bind(past)
            .bind(chain.gen1.refresh_token_hash.expose().clone())
            .execute(&repo.pool)
            .await
            .expect("expire live generation");
        sqlx::query(
            "UPDATE retired_refresh_tokens SET expires_at = $1 WHERE refresh_token_hash = $2",
        )
        .bind(past)
        .bind(chain.gen0.refresh_token_hash.expose().clone())
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
    #[ignore] // Requires a live Postgres: see `test_database_url`.
    async fn revoke_all_user_sessions_sweeps_retired_records_of_that_user_only() {
        let repo = create_isolated_schema_repo("oidc_pg_revoke_all").await;
        seed_fixture_users(&repo, &["usr_mine", "usr_theirs"]).await;
        let mine = session_contract::family_chain("postgres:revoke-all", 0, "usr_mine");
        let theirs = session_contract::family_chain("postgres:revoke-all", 1, "usr_theirs");
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
        assert_eq!(retired_count(&repo).await, 2, "one record per rotation");

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
        assert_eq!(
            retired_count(&repo).await,
            1,
            "only the other user's retirement record remains"
        );
    }

    /// The migration upgrades a `sessions` table that predates the family
    /// columns without touching its rows, is safe to re-run, and legacy rows
    /// read back through the domain type with the sentinel values.
    #[tokio::test]
    #[ignore] // Requires a live Postgres: see `test_database_url`.
    async fn session_family_column_migration_upgrades_legacy_table_and_is_idempotent() {
        let repo = create_isolated_schema_repo("oidc_pg_family_migration").await;

        // Simulate the pre-rotation schema: drop the family columns (and any
        // data depending on them being set), then re-run the idempotent
        // migration statements twice.
        sqlx::query(
            "INSERT INTO users (id, external_id, provider) \
             VALUES ('usr_pg_legacy', 'external|pg_legacy', 'google')",
        )
        .execute(&repo.pool)
        .await
        .expect("seed user for legacy row");
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO sessions (refresh_token_hash, user_id, provider, expires_at, created_at) \
             VALUES ('legacy-hash', 'usr_pg_legacy', 'google', $1, $2)",
        )
        .bind(now + chrono::Duration::hours(1))
        .bind(now)
        .execute(&repo.pool)
        .await
        .expect("insert legacy-shaped row");
        sqlx::query("ALTER TABLE sessions DROP COLUMN family_id")
            .execute(&repo.pool)
            .await
            .expect("drop family_id to simulate the pre-rotation table");
        sqlx::query("ALTER TABLE sessions DROP COLUMN generation")
            .execute(&repo.pool)
            .await
            .expect("drop generation to simulate the pre-rotation table");
        sqlx::query("ALTER TABLE sessions DROP COLUMN rotated_at")
            .execute(&repo.pool)
            .await
            .expect("drop rotated_at to simulate the pre-rotation table");

        for _ in 0..2 {
            sqlx::raw_sql(MIGRATIONS)
                .execute(&repo.pool)
                .await
                .expect("re-running the migration block should not error");
        }

        let legacy = repo
            .get_session_by_refresh_token(&Secret::new("legacy-hash".to_string()))
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
        let canonical = session_contract::generation_session(
            "usr_pg_legacy",
            &session_contract::fixture_family_id("postgres-migration:new-row"),
            0,
            "canonical-hash".to_string(),
            now + chrono::Duration::hours(1),
            now,
            None,
        );
        repo.store_refresh_token(&canonical)
            .await
            .expect("store canonical row post-migration");
        assert_eq!(
            repo.resolve_refresh_token(canonical.refresh_token_hash.expose())
                .await
                .expect("resolve canonical row"),
            RefreshResolution::Live(canonical),
            "the upgraded table must accept and classify canonical rows"
        );
    }

    // -- Single-use conformance (shared suite in test-utils) --------------------

    use oidc_exchange_test_utils::single_use_conformance as conformance;

    #[tokio::test]
    #[ignore] // Requires a live Postgres: see `test_database_url`.
    async fn single_use_first_claim_wins_duplicate_loses() {
        let repo = create_test_repo().await;
        conformance::first_claim_wins_duplicate_loses(&repo).await;
    }

    #[tokio::test]
    #[ignore] // Requires a live Postgres: see `test_database_url`.
    async fn single_use_consume_live_record_exactly_once() {
        let repo = create_test_repo().await;
        conformance::consume_live_record_exactly_once(&repo).await;
    }

    #[tokio::test]
    #[ignore] // Requires a live Postgres: see `test_database_url`.
    async fn single_use_expired_record_is_absent_to_put_and_take() {
        let repo = create_test_repo().await;
        conformance::expired_record_is_absent_to_put_and_take(&repo).await;
    }

    #[tokio::test]
    #[ignore] // Requires a live Postgres: see `test_database_url`.
    async fn single_use_concurrent_put_has_exactly_one_winner() {
        let repo = std::sync::Arc::new(create_test_repo().await);
        conformance::concurrent_put_has_exactly_one_winner(repo).await;
    }

    #[tokio::test]
    #[ignore] // Requires a live Postgres: see `test_database_url`.
    async fn single_use_concurrent_take_has_exactly_one_winner() {
        let repo = std::sync::Arc::new(create_test_repo().await);
        conformance::concurrent_take_has_exactly_one_winner(repo).await;
    }

    #[tokio::test]
    #[ignore] // Requires a live Postgres: see `test_database_url`.
    async fn single_use_cleanup_sweeps_expired_records_and_counts_both_kinds() {
        let repo = create_test_repo().await;
        conformance::cleanup_sweeps_expired_single_use_records(&repo).await;
    }
}
