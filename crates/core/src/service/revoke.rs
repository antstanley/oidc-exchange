use sha2::{Digest, Sha256};
use std::collections::HashMap;

use crate::domain::{
    is_valid_family_id, AuditFailure, AuditOutcome, ClientAddr, SecurityEvent,
};
use crate::error::Result;
use crate::service::AppService;

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
    /// RFC 7009 revocation. Token-state failures always succeed toward the
    /// client (`Ok(())`); only repository/audit infrastructure failures
    /// propagate, which the server maps to 503.
    ///
    /// Revocation authority comes from the credential presented and reaches
    /// exactly the token family or session it names; every resolved terminal
    /// path emits exactly one mandatory, fixed-classification audit outcome.
    /// The mandatory contract applies identically to accepted and rejected
    /// tokens, so an enforce-mode audit failure cannot become an existence
    /// oracle.
    pub async fn revoke(&self, request: RevokeRequest) -> Result<()> {
        // Precondition: an empty token can never verify or hash to a real
        // session, but a caller-supplied empty string is still a programmer
        // error further up the stack (the HTTP form field is required).
        assert!(!request.token.is_empty(), "revoke: token must not be empty");

        let client_addr = request
            .ip_address
            .clone()
            .and_then(ClientAddr::asserted)
            .unwrap_or(ClientAddr::Unknown);
        let user_agent = request.user_agent.clone();

        match request.token_type_hint.as_deref() {
            Some("access_token") => {
                self.revoke_access_token(&request, client_addr, user_agent)
                    .await
            }
            // Unknown hints are treated as refresh tokens per RFC 7009 §2.1.
            Some("refresh_token") | Some(_) | None => {
                self.revoke_refresh_token(&request, client_addr, user_agent)
                    .await
            }
        }
    }

    /// Validate once through the first-party validator, then revoke exactly
    /// the one session family the token's `sid` names. The token's `sub` is
    /// never consulted for authority: a stateless access token is not a
    /// session credential for its whole subject.
    async fn revoke_access_token(
        &self,
        request: &RevokeRequest,
        client_addr: ClientAddr,
        user_agent: Option<String>,
    ) -> Result<()> {
        let claims = match self.validate_access_token(&request.token).await {
            Ok(claims) => claims,
            Err(_reason) => {
                return self
                    .emit_revoke_rejection(client_addr, user_agent)
                    .await;
            }
        };

        // Postconditions of `validate_access_token` needed at this boundary:
        // revocation would otherwise target nothing (or worse, an empty key).
        assert!(
            !claims.sid.is_empty(),
            "revoke: validated access token must carry a non-empty sid"
        );

        // A validated token whose `sid` is not a well-formed family id cannot
        // name a family (a pre-rotation legacy token whose sentinel family is
        // empty, or an interim hash-form sid): reject audibly, mutate
        // nothing.
        if !is_valid_family_id(&claims.sid) {
            return self.emit_revoke_rejection(client_addr, user_agent).await;
        }

        let family_id = claims.sid;
        let user_id = claims.sub;
        assert!(
            !user_id.is_empty(),
            "revoke: verified token sub claim must not be empty"
        );

        let sessions_revoked = self.session_repo.revoke_family(&family_id).await?;

        // One family is one sign-in: the terminal record is the session-scoped
        // TokenRevocation, enriched with correlation detail (never a token
        // hash), emitted through the mandatory durability-governed path.
        let mut event = SecurityEvent::SessionRevoked.into_audit_event(
            AuditOutcome::Success,
            Some(user_id),
            None,
            client_addr,
            user_agent,
        );
        event.detail = HashMap::from([
            ("family_id".to_string(), serde_json::Value::from(family_id)),
            (
                "sessions_revoked".to_string(),
                serde_json::Value::from(sessions_revoked),
            ),
        ]);
        self.emit_mandatory_audit_event(event).await
    }

    /// Hash and revoke a presented refresh token. Every resolved terminal path
    /// emits exactly one mandatory, fixed-classification audit outcome.
    /// Refresh-token revocation stays hash/session-scoped and distinct from
    /// the family-scoped access-token arm above.
    async fn revoke_refresh_token(
        &self,
        request: &RevokeRequest,
        client_addr: ClientAddr,
        user_agent: Option<String>,
    ) -> Result<()> {
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

        match existing {
            Some(session) => {
                // Invariants at the core-to-adapter boundary: every stored
                // session carries the user it belongs to (the audit event
                // needs a real actor, not a blank one), and a keyed lookup
                // returns that very row rather than some other session's.
                assert!(
                    !session.user_id.is_empty(),
                    "revoke: stored session must have a non-empty user_id"
                );
                assert_eq!(
                    session.refresh_token_hash, token_hash,
                    "revoke: hash-keyed lookup must return the matching session"
                );
                // A genuine backend failure here propagates so the server maps
                // it to 503 instead of a false 200.
                self.session_repo.revoke_session(&token_hash).await?;
                self.emit_security_event(
                    SecurityEvent::SessionRevoked,
                    AuditOutcome::Success,
                    Some(session.user_id),
                    None,
                    client_addr,
                    user_agent,
                )
                .await
            }
            None => {
                // Keep the RFC 7009 response indistinguishable even when an
                // enforce-mode audit backend is unavailable: the same
                // mandatory contract applies to known and unknown tokens, and
                // the HTTP layer renders a durability failure identically on
                // both branches.
                self.emit_revoke_rejection(client_addr, user_agent).await
            }
        }
    }

    /// The single rejected-credential record: a fixed-classification
    /// `ValidationFailed` at the mandatory path, actor `None` because no claim
    /// of an unvalidated token is trustworthy enough to record, then `Ok`
    /// toward the client (RFC 7009 §2.2 indistinguishability). Only an
    /// enforce-mode durability failure propagates — identically to the
    /// success branches, so neither becomes an existence oracle.
    async fn emit_revoke_rejection(
        &self,
        client_addr: ClientAddr,
        user_agent: Option<String>,
    ) -> Result<()> {
        self.emit_security_event(
            SecurityEvent::AuthenticationFailed,
            AuditOutcome::Failure(AuditFailure::AuthenticationFailed),
            None,
            None,
            client_addr,
            user_agent,
        )
        .await
    }
}
