use chrono::Utc;
use sha2::{Digest, Sha256};

use crate::domain::{TokenResponse, UserStatus};
use crate::error::{Error, Result};
use crate::service::AppService;

pub struct RefreshRequest {
    pub refresh_token: String,
}

impl AppService {
    pub async fn refresh(&self, request: RefreshRequest) -> Result<TokenResponse> {
        // 1. Hash the presented refresh token (SHA-256, hex-encoded)
        let token_hash = hex::encode(Sha256::digest(request.refresh_token.as_bytes()));

        // 2. Look up session by refresh token hash
        let session = self
            .session_repo
            .get_session_by_refresh_token(&token_hash)
            .await?
            .ok_or_else(|| Error::InvalidToken {
                reason: "unknown refresh token".to_string(),
            })?;

        // 3. Check if the session has expired
        if session.expires_at < Utc::now() {
            return Err(Error::InvalidToken {
                reason: "refresh token expired".to_string(),
            });
        }

        // 4. Look up the user and check status
        let user = self
            .user_repo
            .get_user_by_id(&session.user_id)
            .await?
            .ok_or_else(|| Error::InvalidToken {
                reason: "user not found".to_string(),
            })?;

        if user.status != UserStatus::Active {
            return Err(Error::UserSuspended {
                user_id: user.id,
            });
        }

        // 5. Build and sign a new access token JWT (shared logic)
        let (access_token, expires_in) = self.build_access_token(&user).await?;

        // 6. Return response (no new refresh token on refresh)
        Ok(TokenResponse {
            access_token,
            refresh_token: None,
            token_type: "Bearer".to_string(),
            expires_in,
        })
    }
}
