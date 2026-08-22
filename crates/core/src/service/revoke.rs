use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

use crate::domain::{
    is_valid_family_id, AccessTokenClaims, AuditEventType, AuditOutcome, AuditSeverity,
};
use crate::error::Result;
use crate::service::{create_audit_event, AppService};

/// Length in hex characters of a SHA-256 digest (32 bytes -> 64 hex chars).
/// Named so the `revoke_refresh_token` postcondition assertion documents its
/// bound instead of embedding a magic number.
const TOKEN_HASH_HEX_LEN: usize = 64;

/// Fixed rejection reason for a validly-signed access token whose `sid`
/// cannot name a token family — including pre-change tokens carrying a
/// 64-hex refresh-token hash. Failing closed here is deliberate: passing a
/// hash-valued `sid` onward would "revoke" a family that does not exist,
/// audit a removal that removed nothing, and hide the miss.
///
/// VENDORED SEAM (task 08): PR #19 (`validate_revoke_token_claims`) replaces
/// this fixed-string + `ValidationFailed` shape with its full validator and
/// `AuthenticationFailed` event; only this slice is vendored on this branch.
const SID_REJECTION_REASON: &str =
    "access token sid claim is not a well-formed session family identifier";

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

/// What a presented access token resolves to for revocation purposes.
enum AccessRevokeTarget {
    /// Signature verified and the claims carry a well-formed family `sid`:
    /// revoke exactly that family.
    Family { user_id: String, family_id: String },
    /// Signature verified but the token cannot name a family (missing sid,
    /// malformed sid, hash-form pre-rotation sid): fail closed loudly — one
    /// fixed-reason audit event — while revoking nothing.
    FailClosed,
    /// Verification failed outright (malformed shape, bad signature): RFC
    /// 7009 silence, no audit, no mutation.
    Unverified,
}

impl AppService {
    pub async fn revoke(&self, request: RevokeRequest) -> Result<()> {
        // Precondition: an empty token can never verify or hash to a real
        // session, but a caller-supplied empty string is still a programmer
        // error further up the stack (the HTTP form field is required).
        assert!(!request.token.is_empty(), "revoke: token must not be empty");

        match request.token_type_hint.as_deref() {
            Some("access_token") => self.revoke_access_token(request).await,
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

    /// The access-token arm: resolve the token's stable family identity and
    /// remove precisely that family's live generation and retained retirement
    /// records.
    ///
    /// VENDORED SEAM (task 08): this replaces the interim
    /// `revoke_all_user_sessions(sub)` behaviour per the rotation source
    /// spec's Revocation block. PR #19's validated-claims contract supersedes
    /// the hand-rolled extraction in [`Self::verify_revocation_target`] at
    /// merge time; the client-visible RFC 7009 behaviour below does not
    /// change there or here — token-state outcomes stay silent 200s, backend
    /// failures propagate as errors.
    async fn revoke_access_token(&self, request: RevokeRequest) -> Result<()> {
        match self.verify_revocation_target(&request.token).await {
            AccessRevokeTarget::Unverified => {}
            AccessRevokeTarget::FailClosed => {
                // One fixed-reason rejection, emitted like every other
                // validation failure (Debug severity, dropped under the
                // default emit threshold). Nothing is revoked.
                self.emit_audit(create_audit_event(
                    AuditEventType::ValidationFailed,
                    AuditSeverity::Debug,
                    AuditOutcome::Failure {
                        reason: SID_REJECTION_REASON.to_string(),
                    },
                    None,
                    None,
                    request.ip_address.clone(),
                    request.user_agent.clone(),
                ))
                .await?;
            }
            AccessRevokeTarget::Family { user_id, family_id } => {
                // Paired boundary check: verify_revocation_target already
                // enforced the fam_-form; re-asserting keeps this branch safe
                // independent of that helper's internals.
                assert!(
                    is_valid_family_id(&family_id),
                    "revoke: verified sid must be a well-formed family id"
                );
                assert!(
                    !user_id.is_empty(),
                    "revoke: verified token sub claim must not be empty"
                );

                let sessions_revoked = self.session_repo.revoke_family(&family_id).await?;

                let mut event = create_audit_event(
                    AuditEventType::TokenRevocation,
                    AuditSeverity::Info,
                    AuditOutcome::Success,
                    Some(user_id),
                    None,
                    request.ip_address.clone(),
                    request.user_agent.clone(),
                );
                event.detail = HashMap::from([
                    ("family_id".to_string(), serde_json::Value::from(family_id)),
                    (
                        "sessions_revoked".to_string(),
                        serde_json::Value::from(sessions_revoked),
                    ),
                ]);
                self.emit_audit(event).await?;
            }
        }
        Ok(())
    }

    /// Verify signature and shape, then resolve what the token may revoke.
    ///
    /// Only liveness of the *signature* decides silence vs fail-closed: an
    /// unverifiable token stays fully silent per RFC 7009, while a verifiable
    /// one whose `sid` cannot name a family is rejected audibly but mutates
    /// nothing. The typed [`AccessTokenClaims`] deserialization is itself the
    /// fail-closed gate for payloads missing `sid`.
    async fn verify_revocation_target(&self, token: &str) -> AccessRevokeTarget {
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            return AccessRevokeTarget::Unverified;
        }

        let signing_input = format!("{}.{}", parts[0], parts[1]);
        let Ok(signature_bytes) = URL_SAFE_NO_PAD.decode(parts[2]) else {
            return AccessRevokeTarget::Unverified;
        };

        // Verify signature using the service's key manager; a genuine backend
        // failure surfaces as `Ok(false)`-equivalent silence here because the
        // RFC 7009 carve-out treats verification failure as unknown-token.
        let Ok(valid) = self
            .keys
            .verify(signing_input.as_bytes(), &signature_bytes)
            .await
        else {
            return AccessRevokeTarget::Unverified;
        };
        if !valid {
            return AccessRevokeTarget::Unverified;
        }

        // Signature verified — the payload is ours, so its shape is a
        // programmer/upgrade-window concern, not an attack: parse it typed
        // and fail closed on anything that cannot name a family.
        let Ok(payload_bytes) = URL_SAFE_NO_PAD.decode(parts[1]) else {
            return AccessRevokeTarget::FailClosed;
        };
        let claims: AccessTokenClaims = match serde_json::from_slice(&payload_bytes) {
            Ok(claims) => claims,
            Err(_) => return AccessRevokeTarget::FailClosed,
        };
        if !is_valid_family_id(&claims.sid) || claims.sub.is_empty() {
            return AccessRevokeTarget::FailClosed;
        }

        AccessRevokeTarget::Family {
            user_id: claims.sub,
            family_id: claims.sid,
        }
    }

    /// Hash and revoke a presented refresh token, emitting `TokenRevocation`
    /// only when a session actually matched the hash. `revoke_session` on the
    /// `SessionRepository` port is idempotent and always returns `Ok(())`
    /// even when nothing matched, so the store is queried first to learn
    /// whether a session was really removed — an unknown token must stay
    /// silent per RFC 7009. Refresh-token revocation stays hash/session-scoped
    /// and distinct from the family-scoped access-token arm above.
    async fn revoke_refresh_token(&self, request: &RevokeRequest) -> Result<()> {
        let token_hash = hex::encode(Sha256::digest(request.token.as_bytes()));
        // Postcondition of SHA-256 hex-encoding: always exactly 64 hex
        // characters. Catching a malformed hash here — before it reaches the
        // store — turns a silent lookup miss into a loud programmer error.
        assert_eq!(
            token_hash.len(),
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
}
