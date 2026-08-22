use sha2::{Digest, Sha256};

use crate::domain::{AuditEventType, AuditOutcome, AuditSeverity};
use crate::error::Result;
use crate::service::{create_audit_event, AppService};

/// Length in hex characters of a SHA-256 digest (32 bytes -> 64 hex chars).
/// Named so the hash postcondition below documents its bound instead of
/// embedding a magic number.
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
    /// exactly the session that credential names — never every session of a
    /// subject, and never on the word of an unvalidated token.
    pub async fn revoke(&self, request: RevokeRequest) -> Result<()> {
        // Precondition: an empty token can never verify or hash to a real
        // session, but a caller-supplied empty string is still a programmer
        // error further up the stack (the HTTP form field is required).
        assert!(!request.token.is_empty(), "revoke: token must not be empty");

        match request.token_type_hint.as_deref() {
            Some("access_token") => self.revoke_access_token(&request).await,
            // Unknown hints are treated as refresh tokens per RFC 7009 §2.1.
            Some("refresh_token") | Some(_) | None => self.revoke_refresh_token(&request).await,
        }
    }

    /// Validate once through the first-party validator, then revoke exactly
    /// the one session the token's `sid` names. The token's `sub` is never
    /// consulted for authority: a stateless access token is not a session
    /// credential for its whole subject.
    async fn revoke_access_token(&self, request: &RevokeRequest) -> Result<()> {
        let claims = match self.validate_access_token(&request.token).await {
            Ok(claims) => claims,
            Err(reason) => return self.emit_rejection(request, reason).await,
        };

        // Postconditions of `validate_access_token` needed at this boundary:
        // revocation would otherwise target nothing (or worse, an empty key).
        assert!(
            !claims.sid.is_empty(),
            "revoke: validated access token must carry a non-empty sid"
        );

        self.revoke_one_session(&claims.sid, request).await
    }

    /// Hash and revoke a presented refresh token.
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

        self.revoke_one_session(&token_hash, request).await
    }

    /// Shared revocation tail: look the session up by hash, revoke it, and
    /// audit `TokenRevocation`. A missing session is `Ok` and silent — both
    /// paths are idempotent deletes per RFC 7009 — while lookup/revoke backend
    /// errors propagate so the server maps them to 503 instead of a false 200.
    async fn revoke_one_session(&self, session_hash: &str, request: &RevokeRequest) -> Result<()> {
        let existing = self
            .session_repo
            .get_session_by_refresh_token(session_hash)
            .await?;

        if let Some(session) = existing {
            // Invariants at the core-to-adapter boundary: every stored
            // session carries the user it belongs to (the audit event needs a
            // real actor, not a blank one), and a keyed lookup returns that
            // very row rather than some other session's.
            assert!(
                !session.user_id.is_empty(),
                "revoke: stored session must have a non-empty user_id"
            );
            assert_eq!(
                session.refresh_token_hash, session_hash,
                "revoke: hash-keyed lookup must return the matching session"
            );
            // A genuine backend failure here propagates so the server maps it
            // to 503 instead of a false 200.
            self.session_repo.revoke_session(session_hash).await?;
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

    /// Record a rejected credential with exactly one fixed-reason
    /// `ValidationFailed` event, then succeed toward the client: RFC 7009 §2.2
    /// makes rejected and accepted tokens indistinguishable at the endpoint,
    /// but the attempt stays visible to operators. The reason is a fixed
    /// constant from the validator — never token bytes or decoded content —
    /// and the actor stays `None` because no claim of an unvalidated token is
    /// trustworthy enough to record.
    async fn emit_rejection(&self, request: &RevokeRequest, reason: &'static str) -> Result<()> {
        assert!(
            !reason.is_empty(),
            "revoke: rejection reasons are non-empty constants"
        );

        // Info severity mirrors the success-path `TokenRevocation` emission,
        // so success and failure share identical durability semantics under
        // any audit config — neither branch can become an existence oracle
        // when the audit sink is degraded.
        self.emit_audit(create_audit_event(
            AuditEventType::ValidationFailed,
            AuditSeverity::Info,
            AuditOutcome::Failure {
                reason: reason.to_string(),
            },
            None,
            None,
            request.ip_address.clone(),
            request.user_agent.clone(),
        ))
        .await
    }
}
