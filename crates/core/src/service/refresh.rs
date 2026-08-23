use chrono::Utc;
use sha2::{Digest, Sha256};

use crate::domain::{AuditEventType, AuditOutcome, AuditSeverity, TokenResponse, UserStatus};
use crate::error::{Error, Result};
use crate::service::{create_audit_event, AppService};

#[derive(Default)]
pub struct RefreshRequest {
    pub refresh_token: String,
    /// Client IP address extracted by the server's audit-context middleware.
    pub ip_address: Option<String>,
    /// Client `User-Agent` header, extracted by the server's audit-context
    /// middleware.
    pub user_agent: Option<String>,
    /// Client-supplied device identifier (`X-Device-Id`), extracted by the
    /// server's audit-context middleware.
    pub device_id: Option<String>,
}

impl AppService {
    pub async fn refresh(&self, request: RefreshRequest) -> Result<TokenResponse> {
        // 1. Hash the presented refresh token (SHA-256, hex-encoded). The digest is the
        // session lookup key, so it is wrapped before crossing the repository port.
        let token_hash = crate::secret::Secret::new(hex::encode(Sha256::digest(
            request.refresh_token.as_bytes(),
        )));

        // 2. Look up session by refresh token hash
        let session = match self
            .session_repo
            .get_session_by_refresh_token(&token_hash)
            .await?
        {
            Some(session) => session,
            None => {
                let reason = "unknown refresh token".to_string();
                self.emit_audit(create_audit_event(
                    AuditEventType::ValidationFailed,
                    AuditSeverity::Debug,
                    AuditOutcome::Failure {
                        reason: reason.clone(),
                    },
                    None,
                    None,
                    request.ip_address.clone(),
                    request.user_agent.clone(),
                ))
                .await?;
                return Err(Error::InvalidToken { reason });
            }
        };

        // 3. Check if the session has expired
        if session.expires_at < Utc::now() {
            let reason = "refresh token expired".to_string();
            self.emit_audit(create_audit_event(
                AuditEventType::ValidationFailed,
                AuditSeverity::Debug,
                AuditOutcome::Failure {
                    reason: reason.clone(),
                },
                Some(session.user_id.clone()),
                None,
                request.ip_address.clone(),
                request.user_agent.clone(),
            ))
            .await?;
            return Err(Error::InvalidToken { reason });
        }

        // 4. Look up the user and check status
        let user = match self.user_repo.get_user_by_id(&session.user_id).await? {
            Some(user) => user,
            None => {
                let reason = "user not found".to_string();
                self.emit_audit(create_audit_event(
                    AuditEventType::ValidationFailed,
                    AuditSeverity::Debug,
                    AuditOutcome::Failure {
                        reason: reason.clone(),
                    },
                    Some(session.user_id.clone()),
                    None,
                    request.ip_address.clone(),
                    request.user_agent.clone(),
                ))
                .await?;
                return Err(Error::InvalidToken { reason });
            }
        };

        if user.status != UserStatus::Active {
            self.emit_audit(create_audit_event(
                AuditEventType::UserSuspended,
                AuditSeverity::Warning,
                AuditOutcome::Failure {
                    reason: format!("user status is {:?}, not active", user.status),
                },
                Some(user.id.clone()),
                None,
                request.ip_address.clone(),
                request.user_agent.clone(),
            ))
            .await?;
            return Err(Error::UserSuspended { user_id: user.id });
        }

        // 5. Build and sign a new access token JWT (shared logic)
        let (access_token, expires_in) = self.build_access_token(&user).await?;

        // 6. Audit the successful refresh, after the access token is built.
        self.emit_audit(create_audit_event(
            AuditEventType::TokenRefresh,
            AuditSeverity::Info,
            AuditOutcome::Success,
            Some(user.id.clone()),
            None,
            request.ip_address.clone(),
            request.user_agent.clone(),
        ))
        .await?;

        // 7. Return response (no new refresh token on refresh)
        Ok(TokenResponse {
            access_token,
            refresh_token: None,
            token_type: "Bearer".to_string(),
            expires_in,
        })
    }
}
