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
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_users_external_id ON users (external_id);

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
        email: row.try_get("email").map_err(PostgresRepository::store_err)?,
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

    #[instrument(skip(self), fields(external_id))]
    async fn get_user_by_external_id(&self, external_id: &str) -> Result<Option<User>> {
        let row = sqlx::query("SELECT * FROM users WHERE external_id = $1")
            .bind(external_id)
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
        let metadata = serde_json::to_value(HashMap::<String, Value>::new())
            .map_err(Self::store_err)?;
        let claims = serde_json::to_value(HashMap::<String, Value>::new())
            .map_err(Self::store_err)?;
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

        let metadata_json =
            serde_json::to_value(&user.metadata).map_err(Self::store_err)?;
        let claims_json =
            serde_json::to_value(&user.claims).map_err(Self::store_err)?;
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

    #[instrument(skip(self), fields(user_id))]
    async fn revoke_all_user_sessions(&self, user_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM sessions WHERE user_id = $1")
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(Self::store_err)?;

        Ok(())
    }
}
