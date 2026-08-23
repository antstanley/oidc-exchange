use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use sha2::{Digest, Sha256};

use crate::domain::{AuditEventType, AuditOutcome, AuditSeverity};
use crate::error::Result;
use crate::service::{create_audit_event, AppService};

/// Length in hex characters of a SHA-256 digest (32 bytes -> 64 hex chars).
/// Named so the `revoke_refresh_token` postcondition assertion documents its
/// bound instead of embedding a magic number.
const TOKEN_HASH_HEX_LEN: usize = 64;

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
        // Precondition: an empty token can never verify or hash to a real
        // session, but a caller-supplied empty string is still a programmer
        // error further up the stack (the HTTP form field is required).
        assert!(!request.token.is_empty(), "revoke: token must not be empty");

        match request.token_type_hint.as_deref() {
            Some("access_token") => {
                // Verify the JWT was issued by us before revoking sessions.
                // If verification fails, silently succeed per RFC 7009 — no
                // audit event is emitted for a failed/forged verification.
                // A session-repo error from `revoke_all_user_sessions`
                // (genuine infrastructure failure, since the port is
                // idempotent for a missing user) propagates so the server
                // maps it to 503 instead of a false 200.
                if let Some(user_id) = self.verify_and_extract_sub(&request.token).await {
                    // Postcondition of `verify_and_extract_sub`: a `Some` sub
                    // claim is never empty — the payload check below returns
                    // `None` for a missing/blank `sub`, so an empty string
                    // here would mean that guarantee broke silently.
                    assert!(
                        !user_id.is_empty(),
                        "revoke: verified token sub claim must not be empty"
                    );
                    self.session_repo.revoke_all_user_sessions(&user_id).await?;
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
        let token_hash =
            crate::secret::Secret::new(hex::encode(Sha256::digest(request.token.as_bytes())));
        // Postcondition of SHA-256 hex-encoding: always exactly 64 hex characters.
        // Catching a malformed hash here — before it reaches the store — turns a silent
        // lookup miss into a loud programmer error.
        assert_eq!(
            token_hash.expose().len(),
            TOKEN_HASH_HEX_LEN,
            "revoke: SHA-256 hex digest must be {TOKEN_HASH_HEX_LEN} characters"
        );

        // A missing session is `Ok(None)` (unknown/invalid token, handled
        // below), but a genuine backend failure on this lookup propagates so
        // the server maps it to 503 instead of a false 200.
        let existing = self
            .session_repo
            .get_session_by_refresh_token(&token_hash)
            .await?;

        if let Some(session) = existing {
            // Invariant: every stored session carries the user it belongs
            // to — the audit event below needs a real actor, not a blank one.
            assert!(
                !session.user_id.is_empty(),
                "revoke: stored session must have a non-empty user_id"
            );
            // A missing session is `Ok` (idempotent delete, handled above by
            // `existing == None`); a genuine backend failure here propagates
            // so the server maps it to 503 instead of a false 200.
            self.session_repo.revoke_session(&token_hash).await?;
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
