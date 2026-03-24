pub mod exchange;
pub mod refresh;

use std::collections::HashMap;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::Utc;

use crate::config::AppConfig;
use crate::domain::{AccessTokenClaims, User};
use crate::error::{Error, Result};
use crate::ports::{AuditLog, IdentityProvider, KeyManager, Repository, UserSync};

pub struct AppService {
    pub(crate) repo: Box<dyn Repository>,
    pub(crate) keys: Box<dyn KeyManager>,
    pub(crate) audit: Box<dyn AuditLog>,
    pub(crate) user_sync: Box<dyn UserSync>,
    pub(crate) providers: HashMap<String, Box<dyn IdentityProvider>>,
    pub(crate) config: AppConfig,
}

impl AppService {
    pub fn new(
        repo: Box<dyn Repository>,
        keys: Box<dyn KeyManager>,
        audit: Box<dyn AuditLog>,
        user_sync: Box<dyn UserSync>,
        providers: HashMap<String, Box<dyn IdentityProvider>>,
        config: AppConfig,
    ) -> Self {
        Self {
            repo,
            keys,
            audit,
            user_sync,
            providers,
            config,
        }
    }

    /// Build and sign an access token JWT for the given user.
    ///
    /// Returns `(jwt_string, expires_in_seconds)`.
    pub(crate) async fn build_access_token(&self, user: &User) -> Result<(String, u64)> {
        let now = Utc::now();
        let access_ttl_secs = parse_duration_secs(&self.config.token.access_token_ttl)?;

        let access_claims = AccessTokenClaims {
            sub: user.id.clone(),
            iss: self.config.server.issuer.clone(),
            aud: self.config.token.audience.clone().unwrap_or_default(),
            iat: now.timestamp() as u64,
            exp: (now.timestamp() as u64) + access_ttl_secs,
            custom: HashMap::new(), // custom claims resolution is Task 11
        };

        let claims_json = serde_json::to_vec(&access_claims).map_err(|e| {
            Error::ConfigError {
                detail: format!("failed to serialize access token claims: {}", e),
            }
        })?;

        let header = serde_json::json!({
            "alg": self.keys.algorithm(),
            "typ": "JWT",
            "kid": self.keys.key_id()
        });
        let header_b64 = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&header).map_err(|e| Error::ConfigError {
                detail: format!("failed to serialize JWT header: {}", e),
            })?,
        );
        let payload_b64 = URL_SAFE_NO_PAD.encode(&claims_json);
        let signing_input = format!("{}.{}", header_b64, payload_b64);
        let signature = self.keys.sign(signing_input.as_bytes()).await?;
        let sig_b64 = URL_SAFE_NO_PAD.encode(&signature);
        let access_token = format!("{}.{}", signing_input, sig_b64);

        Ok((access_token, access_ttl_secs))
    }
}

/// Parse a duration string like "15m", "1h", "30d" into seconds.
pub(crate) fn parse_duration_secs(s: &str) -> Result<u64> {
    let s = s.trim();
    if s.is_empty() {
        return Err(Error::ConfigError {
            detail: "empty duration string".to_string(),
        });
    }

    let (num_str, suffix) = s.split_at(s.len() - 1);
    let value: u64 = num_str.parse().map_err(|_| Error::ConfigError {
        detail: format!("invalid duration number: {}", num_str),
    })?;

    match suffix {
        "s" => Ok(value),
        "m" => Ok(value * 60),
        "h" => Ok(value * 3600),
        "d" => Ok(value * 86400),
        _ => Err(Error::ConfigError {
            detail: format!("unknown duration suffix: {}", suffix),
        }),
    }
}
