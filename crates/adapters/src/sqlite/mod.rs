use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
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
    metadata        TEXT NOT NULL DEFAULT '{}',
    claims          TEXT NOT NULL DEFAULT '{}',
    status          TEXT NOT NULL DEFAULT 'active',
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_users_external_id ON users(external_id);

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
        created_at_str.parse().map_err(|e: chrono::ParseError| {
            Error::StoreError {
                detail: format!("failed to parse created_at: {e}"),
            }
        })?;
    let updated_at: DateTime<Utc> =
        updated_at_str.parse().map_err(|e: chrono::ParseError| {
            Error::StoreError {
                detail: format!("failed to parse updated_at: {e}"),
            }
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
        created_at,
        updated_at,
    })
}

fn row_to_session(row: &sqlx::sqlite::SqliteRow) -> Result<Session> {
    let expires_at_str: String = row.get("expires_at");
    let created_at_str: String = row.get("created_at");

    let expires_at: DateTime<Utc> =
        expires_at_str.parse().map_err(|e: chrono::ParseError| {
            Error::StoreError {
                detail: format!("failed to parse expires_at: {e}"),
            }
        })?;
    let created_at: DateTime<Utc> =
        created_at_str.parse().map_err(|e: chrono::ParseError| {
            Error::StoreError {
                detail: format!("failed to parse created_at: {e}"),
            }
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

    #[instrument(skip(self), fields(external_id))]
    async fn get_user_by_external_id(&self, external_id: &str) -> Result<Option<User>> {
        let row = sqlx::query("SELECT * FROM users WHERE external_id = ?1")
            .bind(external_id)
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
        .map_err(|e| Error::StoreError {
            detail: e.to_string(),
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
            created_at: now,
            updated_at: now,
        })
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

        let metadata_str =
            serde_json::to_string(&user.metadata).map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;
        let claims_str =
            serde_json::to_string(&user.claims).map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;
        let status_str = status_to_str(&user.status);
        let updated_at_str = user.updated_at.to_rfc3339();

        sqlx::query(
            "UPDATE users SET email = ?1, display_name = ?2, metadata = ?3, claims = ?4, status = ?5, updated_at = ?6 \
             WHERE id = ?7",
        )
        .bind(&user.email)
        .bind(&user.display_name)
        .bind(&metadata_str)
        .bind(&claims_str)
        .bind(status_str)
        .bind(&updated_at_str)
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::StoreError {
            detail: e.to_string(),
        })?;

        Ok(user)
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
        let rows = sqlx::query(
            "SELECT * FROM users ORDER BY created_at DESC LIMIT ?1 OFFSET ?2",
        )
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

        // Get by ID
        let fetched = repo
            .get_user_by_id(&created.id)
            .await
            .expect("get_user_by_id")
            .expect("user should exist");
        assert_eq!(fetched.id, created.id);

        // Get by external ID
        let fetched_ext = repo
            .get_user_by_external_id("google|user123")
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
        repo.store_refresh_token(&session)
            .await
            .expect("re-store");
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
}
