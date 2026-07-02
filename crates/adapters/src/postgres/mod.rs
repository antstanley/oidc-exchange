use std::collections::HashMap;

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
        .map_err(Self::store_err)?;

        row_to_user(&row)
    }

    #[instrument(skip(self, patch), fields(user_id))]
    async fn update_user(&self, user_id: &str, patch: &UserPatch) -> Result<User> {
        let mut user = self
            .get_user_by_id(user_id)
            .await?
            .ok_or_else(|| Error::StoreError {
                detail: format!("user not found: {user_id}"),
            })?;

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

        let row = sqlx::query(
            "UPDATE users SET email = $1, display_name = $2, metadata = $3, claims = $4, status = $5, updated_at = $6
             WHERE id = $7
             RETURNING *",
        )
        .bind(&user.email)
        .bind(&user.display_name)
        .bind(&metadata_json)
        .bind(&claims_json)
        .bind(status_str)
        .bind(user.updated_at)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await
        .map_err(Self::store_err)?;

        row_to_user(&row)
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
}
