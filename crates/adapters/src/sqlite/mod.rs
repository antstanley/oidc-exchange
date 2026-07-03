use std::collections::HashMap;
use std::future::Future;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use tracing::instrument;

use oidc_exchange_core::domain::{
    NewUser, Session, User, UserPatch, UserStatus, INITIAL_USER_VERSION,
};
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
    provider            TEXT NOT NULL,
    expires_at          TEXT NOT NULL,
    device_id           TEXT,
    user_agent          TEXT,
    ip_address          TEXT,
    created_at          TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_sessions_user_id ON sessions(user_id);
"#;

pub struct SqliteRepository {
    pool: SqlitePool,
}

impl SqliteRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
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

    Ok(Session {
        user_id: row.get("user_id"),
        refresh_token_hash: row.get("refresh_token_hash"),
        provider: row.get("provider"),
        expires_at,
        device_id: row.get("device_id"),
        user_agent: row.get("user_agent"),
        ip_address: row.get("ip_address"),
        created_at,
    })
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
        let expires_at_str = session.expires_at.to_rfc3339();
        let created_at_str = session.created_at.to_rfc3339();

        sqlx::query(
            "INSERT OR REPLACE INTO sessions (refresh_token_hash, user_id, provider, expires_at, device_id, user_agent, ip_address, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )
        .bind(&session.refresh_token_hash)
        .bind(&session.user_id)
        .bind(&session.provider)
        .bind(&expires_at_str)
        .bind(&session.device_id)
        .bind(&session.user_agent)
        .bind(&session.ip_address)
        .bind(&created_at_str)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::StoreError {
            detail: e.to_string(),
        })?;

        Ok(())
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
        sqlx::query("DELETE FROM sessions WHERE user_id = ?1")
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

        Ok(())
    }

    #[instrument(skip(self))]
    async fn cleanup_expired_sessions(&self) -> Result<u64> {
        let now_str = Utc::now().to_rfc3339();
        let result = sqlx::query("DELETE FROM sessions WHERE expires_at < ?1")
            .bind(&now_str)
            .execute(&self.pool)
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

        SqliteRepository::new(pool)
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
            provider: "google".to_string(),
            expires_at: now + chrono::Duration::hours(24),
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
            provider: "google".to_string(),
            expires_at: now + chrono::Duration::hours(24),
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
        let repo = SqliteRepository::new(pool);

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

        let repo = SqliteRepository::new(pool);
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

        (SqliteRepository::new(pool), dir)
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
}
