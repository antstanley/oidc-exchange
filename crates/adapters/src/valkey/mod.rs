use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fred::prelude::*;
use oidc_exchange_core::domain::Session;
use oidc_exchange_core::error::{Error, Result};
use oidc_exchange_core::ports::SessionRepository;
use tracing::instrument;

pub struct ValkeySessionRepository {
    client: fred::clients::Client,
    key_prefix: String,
}

impl ValkeySessionRepository {
    pub async fn new(url: &str, key_prefix: String) -> std::result::Result<Self, Box<dyn std::error::Error>> {
        let config = Config::from_url(url)?;
        let client = fred::clients::Client::new(config, None, None, None);
        client.init().await?;
        Ok(Self { client, key_prefix })
    }

    fn session_key(&self, token_hash: &str) -> String {
        format!("{}session:{}", self.key_prefix, token_hash)
    }

    fn user_sessions_key(&self, user_id: &str) -> String {
        format!("{}user_sessions:{}", self.key_prefix, user_id)
    }
}

#[async_trait]
impl SessionRepository for ValkeySessionRepository {
    #[instrument(skip(self))]
    async fn store_refresh_token(&self, session: &Session) -> Result<()> {
        let key = self.session_key(&session.refresh_token_hash);

        let fields: Vec<(&str, String)> = vec![
            ("user_id", session.user_id.clone()),
            ("refresh_token_hash", session.refresh_token_hash.clone()),
            ("provider", session.provider.clone()),
            ("expires_at", session.expires_at.to_rfc3339()),
            ("device_id", session.device_id.clone().unwrap_or_default()),
            ("user_agent", session.user_agent.clone().unwrap_or_default()),
            ("ip_address", session.ip_address.clone().unwrap_or_default()),
            ("created_at", session.created_at.to_rfc3339()),
        ];

        self.client
            .hset::<(), _, _>(&key, fields)
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

        // Compute TTL from expires_at
        let ttl_seconds = (session.expires_at - Utc::now()).num_seconds();
        if ttl_seconds > 0 {
            self.client
                .expire::<(), _>(&key, ttl_seconds, None)
                .await
                .map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })?;
        }

        // Track this session in the user's session set
        self.client
            .sadd::<(), _, _>(
                self.user_sessions_key(&session.user_id),
                &session.refresh_token_hash,
            )
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

        Ok(())
    }

    #[instrument(skip(self))]
    async fn get_session_by_refresh_token(&self, token_hash: &str) -> Result<Option<Session>> {
        let key = self.session_key(token_hash);

        let values: std::collections::HashMap<String, String> = self
            .client
            .hgetall(&key)
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

        if values.is_empty() {
            return Ok(None);
        }

        let get_field = |name: &str| -> Result<String> {
            values
                .get(name)
                .cloned()
                .ok_or_else(|| Error::StoreError {
                    detail: format!("missing field: {}", name),
                })
        };

        let parse_dt = |s: &str| -> Result<DateTime<Utc>> {
            DateTime::parse_from_rfc3339(s)
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })
        };

        let device_id = values.get("device_id").and_then(|v| {
            if v.is_empty() {
                None
            } else {
                Some(v.clone())
            }
        });
        let user_agent = values.get("user_agent").and_then(|v| {
            if v.is_empty() {
                None
            } else {
                Some(v.clone())
            }
        });
        let ip_address = values.get("ip_address").and_then(|v| {
            if v.is_empty() {
                None
            } else {
                Some(v.clone())
            }
        });

        let expires_at_str = get_field("expires_at")?;
        let created_at_str = get_field("created_at")?;

        Ok(Some(Session {
            user_id: get_field("user_id")?,
            refresh_token_hash: get_field("refresh_token_hash")?,
            provider: get_field("provider")?,
            expires_at: parse_dt(&expires_at_str)?,
            device_id,
            user_agent,
            ip_address,
            created_at: parse_dt(&created_at_str)?,
        }))
    }

    #[instrument(skip(self))]
    async fn revoke_session(&self, token_hash: &str) -> Result<()> {
        let key = self.session_key(token_hash);

        // Get user_id before deleting so we can clean up the user set
        let user_id: Option<String> = self
            .client
            .hget(&key, "user_id")
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

        self.client
            .del::<(), _>(&key)
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

        if let Some(user_id) = user_id {
            self.client
                .srem::<(), _, _>(self.user_sessions_key(&user_id), token_hash)
                .await
                .map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })?;
        }

        Ok(())
    }

    #[instrument(skip(self))]
    async fn revoke_all_user_sessions(&self, user_id: &str) -> Result<()> {
        let user_set_key = self.user_sessions_key(user_id);

        let token_hashes: Vec<String> = self
            .client
            .smembers(&user_set_key)
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

        for token_hash in &token_hashes {
            let key = self.session_key(token_hash);
            self.client
                .del::<(), _>(&key)
                .await
                .map_err(|e| Error::StoreError {
                    detail: e.to_string(),
                })?;
        }

        self.client
            .del::<(), _>(&user_set_key)
            .await
            .map_err(|e| Error::StoreError {
                detail: e.to_string(),
            })?;

        Ok(())
    }
}
