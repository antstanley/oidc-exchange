use std::collections::HashMap;
use std::future::Future;

use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};
use tracing::instrument;

use oidc_exchange_core::domain::{NewUser, Session, User, UserPatch, UserStatus};
use oidc_exchange_core::error::{Error, Result};
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

CREATE UNIQUE INDEX IF NOT EXISTS idx_users_external_id_provider ON users (external_id, provider);

CREATE TABLE IF NOT EXISTS sessions (
    refresh_token_hash  TEXT PRIMARY KEY,
    user_id             TEXT NOT NULL REFERENCES users(id),
    provider            TEXT NOT NULL,
    expires_at          TIMESTAMPTZ NOT NULL,
    device_id           TEXT,
    user_agent          TEXT,
    ip_address          TEXT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_sessions_user_id ON sessions (user_id);
"#;

pub struct PostgresRepository {
    pool: PgPool,
}

impl PostgresRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
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

pub async fn create_pool(
    url: &str,
    max_connections: u32,
) -> std::result::Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(max_connections)
        .connect(url)
        .await
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
    Ok(Session {
        user_id: row
            .try_get("user_id")
            .map_err(PostgresRepository::store_err)?,
        refresh_token_hash: row
            .try_get("refresh_token_hash")
            .map_err(PostgresRepository::store_err)?,
        provider: row
            .try_get("provider")
            .map_err(PostgresRepository::store_err)?,
        expires_at: row
            .try_get("expires_at")
            .map_err(PostgresRepository::store_err)?,
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
        let row = sqlx::query("SELECT * FROM users WHERE external_id = $1 AND provider = $2")
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
        sqlx::query(
            "INSERT INTO sessions (refresh_token_hash, user_id, provider, expires_at, device_id, user_agent, ip_address, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (refresh_token_hash) DO UPDATE SET
                user_id = EXCLUDED.user_id,
                provider = EXCLUDED.provider,
                expires_at = EXCLUDED.expires_at,
                device_id = EXCLUDED.device_id,
                user_agent = EXCLUDED.user_agent,
                ip_address = EXCLUDED.ip_address,
                created_at = EXCLUDED.created_at",
        )
        .bind(&session.refresh_token_hash)
        .bind(&session.user_id)
        .bind(&session.provider)
        .bind(session.expires_at)
        .bind(&session.device_id)
        .bind(&session.user_agent)
        .bind(&session.ip_address)
        .bind(session.created_at)
        .execute(&self.pool)
        .await
        .map_err(Self::store_err)?;

        Ok(())
    }

    #[instrument(skip(self), fields(token_hash))]
    async fn get_session_by_refresh_token(&self, token_hash: &str) -> Result<Option<Session>> {
        let row = sqlx::query("SELECT * FROM sessions WHERE refresh_token_hash = $1")
            .bind(token_hash)
            .fetch_optional(&self.pool)
            .await
            .map_err(Self::store_err)?;

        match row {
            Some(ref r) => Ok(Some(row_to_session(r)?)),
            None => Ok(None),
        }
    }

    #[instrument(skip(self), fields(token_hash))]
    async fn revoke_session(&self, token_hash: &str) -> Result<()> {
        sqlx::query("DELETE FROM sessions WHERE refresh_token_hash = $1")
            .bind(token_hash)
            .execute(&self.pool)
            .await
            .map_err(Self::store_err)?;

        Ok(())
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
        sqlx::query("DELETE FROM sessions WHERE user_id = $1")
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(Self::store_err)?;

        Ok(())
    }

    #[instrument(skip(self))]
    async fn cleanup_expired_sessions(&self) -> Result<u64> {
        let result = sqlx::query("DELETE FROM sessions WHERE expires_at < NOW()")
            .execute(&self.pool)
            .await
            .map_err(Self::store_err)?;

        Ok(result.rows_affected())
    }
}

// ---------------------------------------------------------------------------
// Integration tests (require a live PostgreSQL instance)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use oidc_exchange_core::domain::INITIAL_USER_VERSION;

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
        let pool = create_pool(&test_database_url(), 5)
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

        PostgresRepository::new(pool)
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
        let bootstrap_pool = create_pool(&test_database_url(), 1)
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

        PostgresRepository::new(pool)
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
}
