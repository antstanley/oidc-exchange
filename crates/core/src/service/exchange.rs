use std::collections::HashMap;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::Utc;
use sha2::{Digest, Sha256};

use crate::domain::{
    AccessTokenClaims, NewUser, Session, TokenResponse, UserStatus,
};
use crate::error::{Error, Result};
use crate::service::AppService;

pub struct ExchangeRequest {
    pub code: String,
    pub redirect_uri: String,
    pub provider: String,
}

/// Parse a duration string like "15m", "1h", "30d" into seconds.
fn parse_duration_secs(s: &str) -> Result<u64> {
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

impl AppService {
    pub async fn exchange(&self, request: ExchangeRequest) -> Result<TokenResponse> {
        // 1. Resolve provider
        let provider =
            self.providers
                .get(&request.provider)
                .ok_or_else(|| Error::UnknownProvider {
                    provider: request.provider.clone(),
                })?;

        // 2. Exchange code for provider tokens
        let tokens = provider
            .exchange_code(&request.code, &request.redirect_uri)
            .await?;

        // 3. Validate ID token and extract claims
        let claims = provider.validate_id_token(&tokens.id_token).await?;

        // 4. Look up user by external ID
        let user = match self.repo.get_user_by_external_id(&claims.subject).await? {
            Some(user) => {
                if user.status != UserStatus::Active {
                    return Err(Error::UserSuspended {
                        user_id: user.id,
                    });
                }
                user
            }
            None => {
                // Create new user (registration policy enforcement is Task 7)
                let new_user = NewUser {
                    external_id: claims.subject.clone(),
                    provider: request.provider.clone(),
                    email: claims.email.clone(),
                    display_name: claims.name.clone(),
                };
                self.repo.create_user(&new_user).await?
            }
        };

        // 5. Generate refresh token (256-bit random, base64url-encoded)
        use rand::RngCore;
        let mut token_bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut token_bytes);
        let refresh_token = URL_SAFE_NO_PAD.encode(token_bytes);

        // 6. Hash refresh token with SHA-256 (hex-encoded)
        let token_hash = hex::encode(Sha256::digest(refresh_token.as_bytes()));

        // 7. Compute session expiry from config
        let refresh_ttl_secs =
            parse_duration_secs(&self.config.token.refresh_token_ttl)?;
        let expires_at =
            Utc::now() + chrono::Duration::seconds(refresh_ttl_secs as i64);

        // 8. Store session
        let session = Session {
            user_id: user.id.clone(),
            refresh_token_hash: token_hash,
            provider: request.provider.clone(),
            expires_at,
            device_id: None,
            user_agent: None,
            ip_address: None,
            created_at: Utc::now(),
        };
        self.repo.store_refresh_token(&session).await?;

        // 9. Build access token claims and sign as JWT
        let now = Utc::now();
        let access_ttl_secs =
            parse_duration_secs(&self.config.token.access_token_ttl)?;

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

        Ok(TokenResponse {
            access_token,
            refresh_token: Some(refresh_token),
            token_type: "Bearer".to_string(),
            expires_in: access_ttl_secs,
        })
    }
}

/// Parse a duration string like "15m", "1h", "30d" into seconds. Exposed for
/// unit testing via integration tests.
#[cfg(test)]
mod tests {
    use super::parse_duration_secs;

    #[test]
    fn parse_duration_secs_works() {
        assert_eq!(parse_duration_secs("15m").unwrap(), 900);
        assert_eq!(parse_duration_secs("1h").unwrap(), 3600);
        assert_eq!(parse_duration_secs("30d").unwrap(), 2592000);
        assert_eq!(parse_duration_secs("60s").unwrap(), 60);
        assert!(parse_duration_secs("").is_err());
        assert!(parse_duration_secs("abc").is_err());
        assert!(parse_duration_secs("15x").is_err());
    }
}
