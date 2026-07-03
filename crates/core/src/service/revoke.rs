use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use sha2::{Digest, Sha256};

use crate::domain::{AuditEventType, AuditOutcome, AuditSeverity};
use crate::error::Result;
use crate::service::{create_audit_event, AppService};

#[derive(Default)]
pub struct RevokeRequest {
    pub token: String,
    pub token_type_hint: Option<String>, // "refresh_token" or "access_token"
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
    pub async fn revoke(&self, request: RevokeRequest) -> Result<()> {
        match request.token_type_hint.as_deref() {
            Some("access_token") => {
                // Verify the JWT was issued by us before revoking sessions.
                // If verification fails, silently succeed per RFC 7009 — no
                // audit event is emitted for a failed/forged verification.
                if let Some(user_id) = self.verify_and_extract_sub(&request.token).await {
                    let _ = self.session_repo.revoke_all_user_sessions(&user_id).await;
                    self.emit_audit(create_audit_event(
                        AuditEventType::AllSessionsRevoked,
                        AuditSeverity::Notice,
                        AuditOutcome::Success,
                        Some(user_id),
                        None,
                        request.ip_address.clone(),
                        request.user_agent.clone(),
                    ))
                    .await?;
                }
                Ok(())
            }
            Some("refresh_token") | None => {
                self.revoke_refresh_token(&request).await?;
                Ok(())
            }
            Some(_) => {
                // Unknown hint — treat as refresh_token per spec
                self.revoke_refresh_token(&request).await?;
                Ok(())
            }
        }
    }

    /// Hash and revoke a presented refresh token, emitting `TokenRevocation`
    /// only when a session actually matched the hash. `revoke_session` on the
    /// `SessionRepository` port is idempotent and always returns `Ok(())`
    /// even when nothing matched, so the store is queried first to learn
    /// whether a session was really removed — an unknown token must stay
    /// silent per RFC 7009.
    async fn revoke_refresh_token(&self, request: &RevokeRequest) -> Result<()> {
        let token_hash = hex::encode(Sha256::digest(request.token.as_bytes()));
        let existing = self
            .session_repo
            .get_session_by_refresh_token(&token_hash)
            .await
            .ok()
            .flatten();

        if let Some(session) = existing {
            let _ = self.session_repo.revoke_session(&token_hash).await;
            self.emit_audit(create_audit_event(
                AuditEventType::TokenRevocation,
                AuditSeverity::Info,
                AuditOutcome::Success,
                Some(session.user_id.clone()),
                None,
                request.ip_address.clone(),
                request.user_agent.clone(),
            ))
            .await?;
        }
        Ok(())
    }

    /// Verify a JWT's signature using the service's key manager, then extract the `sub` claim.
    /// Returns None if the token is malformed or the signature is invalid.
    async fn verify_and_extract_sub(&self, token: &str) -> Option<String> {
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            return None;
        }

        let signing_input = format!("{}.{}", parts[0], parts[1]);
        let signature_bytes = URL_SAFE_NO_PAD.decode(parts[2]).ok()?;

        // Verify signature using the service's key manager
        let valid = self
            .keys
            .verify(signing_input.as_bytes(), &signature_bytes)
            .await
            .ok()?;

        if !valid {
            return None;
        }

        // Signature verified — safe to extract sub
        let payload_bytes = URL_SAFE_NO_PAD.decode(parts[1]).ok()?;
        let payload: serde_json::Value = serde_json::from_slice(&payload_bytes).ok()?;
        payload.get("sub")?.as_str().map(|s| s.to_string())
    }
}
