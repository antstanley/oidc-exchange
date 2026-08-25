use std::collections::HashMap;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, Duration, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::config::MAX_REFRESH_ROTATION_GRACE_SECS;
use crate::domain::{
    is_valid_family_id, new_family_id, AuditEventType, AuditFailure, AuditOutcome, AuditSeverity,
    AuthenticationKind, ClientAddr, RefreshResolution, SecurityEvent, Session, TokenResponse,
    UserStatus,
};
use crate::error::{Error, Result};
use crate::service::{create_audit_event, AppService};

/// Reason string carried by every unknown-token refusal — and, deliberately,
/// by the reuse refusal too: telling a presenter that they tripped the reuse
/// detector tells an attacker exactly when to stop. Centralized as a constant
/// so the two branches cannot drift apart.
const UNKNOWN_REFRESH_TOKEN_REASON: &str = "unknown refresh token";

/// Reason string for a live generation whose family's absolute deadline has
/// passed. Rotation never moves `expires_at`, so this is the one refusal a
/// legitimate holder can hit by waiting.
const EXPIRED_REFRESH_TOKEN_REASON: &str = "refresh token expired";

/// Reason string when the session's user no longer resolves.
const MISSING_USER_REASON: &str = "user not found";

/// Length in hex characters of a SHA-256 digest (32 bytes → 64 hex chars).
/// Named so the hashing postcondition asserts its bound instead of embedding
/// a magic number.
const REFRESH_HASH_HEX_LEN: usize = 64;

/// Entropy, in bytes, of every minted refresh token (256-bit). Named per the
/// limits rule so the mint site documents its bound.
const REFRESH_TOKEN_BYTES: usize = 32;

#[derive(Default)]
pub struct RefreshRequest {
    pub refresh_token: String,
    /// Client address with the provenance the server's audit-context middleware
    /// resolved it from (`Peer`/`Forwarded`/`Unknown`), carried so the flow's
    /// audit events record the true `ip_address_source`. Defaults to `Unknown`.
    pub client_addr: ClientAddr,
    /// Client `User-Agent` header, extracted by the server's audit-context
    /// middleware.
    pub user_agent: Option<String>,
    /// Client-supplied device identifier (`X-Device-Id`), extracted by the
    /// server's audit-context middleware.
    pub device_id: Option<String>,
}

impl AppService {
    /// Redeem a refresh token.
    ///
    /// Redemption is a state transition, classified by the store port and
    /// decided here (the port classifies; core owns policy):
    ///
    /// - **Unknown** → refused exactly as before (`ValidationFailed` at
    ///   Debug, then `InvalidToken`).
    /// - **Live** → rotate: expiry/user gates run *before* any write, the
    ///   replacement mints through the port's atomic compare-and-swap, and a
    ///   losing CAS refuses generically without revoking anything.
    /// - **Superseded inside the configured grace window** → rotate forward
    ///   once from the current live generation.
    /// - **Superseded outside grace, or Retired** → reuse: revoke only that
    ///   family, emit `RefreshTokenReuse` at Warning with
    ///   `{family_id, sessions_revoked}`, and return the same unknown-token
    ///   error the presenter would get for a hash that never existed.
    ///
    /// With `[token] refresh_rotation = false` nothing is minted or retired:
    /// leftover retirement classifications are treated as unknown (refused,
    /// no alarm, no revocation) because the switch disables the response along
    /// with the rotation.
    pub async fn refresh(&self, request: RefreshRequest) -> Result<TokenResponse> {
        // Precondition: an empty presented token is a caller bug further up
        // the stack (the HTTP form field is required) — refuse loudly here.
        assert!(
            !request.refresh_token.is_empty(),
            "refresh: presented refresh token must not be empty"
        );

        // 1. Hash the presented token; the digest length postcondition pairs
        // with the store-side lookups that key on it.
        let token_hash = hex::encode(Sha256::digest(request.refresh_token.as_bytes()));
        assert_eq!(
            token_hash.len(),
            REFRESH_HASH_HEX_LEN,
            "refresh: SHA-256 hex digest must be {REFRESH_HASH_HEX_LEN} characters"
        );

        let resolution = self.session_repo.resolve_refresh_token(&token_hash).await?;

        if !self.config.token.refresh_rotation {
            return self.refresh_without_rotation(resolution, &request).await;
        }

        match resolution {
            RefreshResolution::Unknown => {
                self.refuse_with_validation_failed(UNKNOWN_REFRESH_TOKEN_REASON, None, &request)
                    .await
            }
            RefreshResolution::Live(session) => {
                self.rotate_and_respond(session, false, &request).await
            }
            RefreshResolution::Superseded { live, retired_at } => {
                let grace_secs = self.rotation_grace_secs()?;
                if Self::within_grace(retired_at, grace_secs) {
                    self.rotate_and_respond(live, true, &request).await
                } else {
                    self.revoke_family_for_reuse(&live.family_id, &live.user_id, &request)
                        .await
                }
            }
            RefreshResolution::Retired {
                family_id, user_id, ..
            } => {
                self.revoke_family_for_reuse(&family_id, &user_id, &request)
                    .await
            }
        }
    }

    /// Parse the configured grace window in seconds. Startup validation
    /// already bounds it to `(0, MAX_REFRESH_ROTATION_GRACE_SECS]`; the
    /// assertions re-check that invariant at the point of use rather than
    /// trusting config that tests may have built by hand.
    fn rotation_grace_secs(&self) -> Result<u64> {
        let secs = self.config.token.refresh_rotation_grace.as_secs();
        assert!(
            secs > 0 && secs <= MAX_REFRESH_ROTATION_GRACE_SECS,
            "refresh grace {}s must be within (0, {MAX_REFRESH_ROTATION_GRACE_SECS}] after validation",
            secs
        );
        Ok(secs)
    }

    /// Whether a retirement record's `retired_at` still grants grace. A
    /// slightly-future `retired_at` (clock jitter between writer and reader)
    /// compares as inside the window, which is the safe direction: the record
    /// is brand-new, not stale.
    fn within_grace(retired_at: DateTime<Utc>, grace_secs: u64) -> bool {
        let window = Duration::seconds(grace_secs as i64);
        Utc::now() - retired_at <= window
    }

    /// Shared refusal path for unknown / expired / missing-user / disabled-
    /// mode retired presentations: one `ValidationFailed` audit at Debug
    /// (below the default emit threshold), then `InvalidToken` carrying the
    /// given reason. Always returns `Err`.
    async fn refuse_with_validation_failed(
        &self,
        reason: &str,
        actor: Option<String>,
        request: &RefreshRequest,
    ) -> Result<TokenResponse> {
        assert!(
            !reason.is_empty(),
            "refresh refusal must always carry a reason"
        );
        self.emit_audit(create_audit_event(
            AuditEventType::ValidationFailed,
            AuditSeverity::Debug,
            AuditOutcome::Failure(AuditFailure::AuthenticationFailed),
            actor,
            None,
            request.client_addr.clone(),
            request.user_agent.clone(),
        ))
        .await?;
        Err(Error::InvalidToken {
            reason: reason.to_string(),
        })
    }

    /// The reuse branch. Revocation runs **before** the emission so a blocking
    /// audit failure cannot leave the family alive (the emission error then
    /// propagates and fails the request), and the returned error is
    /// byte-identical to the unknown-token refusal so the response does not
    /// tell the presenter that an alarm fired. Always returns `Err`.
    async fn revoke_family_for_reuse(
        &self,
        family_id: &str,
        user_id: &str,
        request: &RefreshRequest,
    ) -> Result<TokenResponse> {
        assert!(
            is_valid_family_id(family_id),
            "reuse revocation requires a well-formed family id, got {family_id:?}"
        );
        assert!(
            !user_id.is_empty(),
            "reuse revocation requires the family's user id"
        );

        let sessions_revoked = self.session_repo.revoke_family(family_id).await?;

        // Detail carries correlation data only — never a token hash or digest.
        let detail: HashMap<String, Value> = HashMap::from([
            ("family_id".to_string(), Value::from(family_id)),
            (
                "sessions_revoked".to_string(),
                Value::from(sessions_revoked),
            ),
        ]);
        assert!(
            !detail
                .values()
                .any(|v| v.as_str().is_some_and(|s| s.len() == REFRESH_HASH_HEX_LEN)),
            "audit detail must never carry a token-hash-shaped value"
        );
        // Reuse is a security outcome: emit on the mandatory channel
        // (`emit_threshold`-immune, `audit.durability`-governed), byte-compatible
        // with the previous best-effort event (`refresh_token_reuse`, warning,
        // outcome `success`, detail `{family_id, sessions_revoked}`) — only the
        // channel changes. Revocation already ran above, so a durability-enforced
        // emission failure cannot leave the reused family alive.
        self.emit_security_event_with_detail(
            SecurityEvent::RefreshTokenReuse,
            AuditOutcome::Success,
            Some(user_id.to_string()),
            None,
            request.client_addr.clone(),
            request.user_agent.clone(),
            detail,
        )
        .await?;

        // Postcondition of the indistinguishability rule: the reuse refusal
        // must carry exactly the unknown-token reason string.
        let err = Error::InvalidToken {
            reason: UNKNOWN_REFRESH_TOKEN_REASON.to_string(),
        };
        assert!(
            matches!(&err, Error::InvalidToken { reason } if reason == UNKNOWN_REFRESH_TOKEN_REASON),
            "reuse must be indistinguishable from an unknown token"
        );
        Err(err)
    }

    /// Mint the next generation of `live`'s family. The replacement inherits
    /// everything identifying the sign-in — user, provider, device context,
    /// `created_at`, and the absolute `expires_at` — and advances the
    /// generation by exactly one. A pre-rotation legacy row (empty-string
    /// family sentinel) gets a freshly minted family on its first redemption:
    /// adapters never synthesize families, the caller does.
    fn mint_replacement(&self, live: &Session) -> Result<(String, Session)> {
        assert!(
            !live.refresh_token_hash.expose().is_empty(),
            "mint_replacement: live generation must carry a hash"
        );

        let raw_token = URL_SAFE_NO_PAD.encode(rand::random::<[u8; REFRESH_TOKEN_BYTES]>());
        let token_hash = hex::encode(Sha256::digest(raw_token.as_bytes()));
        assert_eq!(token_hash.len(), REFRESH_HASH_HEX_LEN);
        assert!(
            token_hash != *live.refresh_token_hash.expose(),
            "a fresh 256-bit generation colliding with the presented hash is a programmer error"
        );

        // Legacy rows read Live with the empty-string sentinel; enabled
        // rotation replaces it with a caller-minted well-formed family id.
        let family_id = if live.family_id.is_empty() {
            new_family_id()
        } else {
            live.family_id.clone()
        };
        assert!(
            is_valid_family_id(&family_id),
            "mint_replacement: replacement family {family_id:?} must be well-formed"
        );

        let replacement = Session {
            user_id: live.user_id.clone(),
            refresh_token_hash: crate::secret::Secret::new(token_hash),
            family_id,
            generation: live.generation + 1,
            provider: live.provider.clone(),
            expires_at: live.expires_at,
            rotated_at: Some(Utc::now()),
            device_id: live.device_id.clone(),
            user_agent: live.user_agent.clone(),
            ip_address: live.ip_address.clone(),
            created_at: live.created_at,
        };

        // Paired postconditions: rotation never slides the absolute deadline
        // nor re-dates the sign-in — recomputing either would convert a
        // bounded session into an unbounded one.
        assert_eq!(
            replacement.expires_at, live.expires_at,
            "replacement must inherit the family's absolute expires_at unchanged"
        );
        assert_eq!(
            replacement.created_at, live.created_at,
            "replacement must inherit the family's original created_at"
        );

        Ok((raw_token, replacement))
    }

    /// Rotate from the live generation and respond with the replacement.
    ///
    /// Ordering matters and matches the source spec: resolve → reuse check →
    /// expiry gate → user gate → mint → atomic swap → sign → audit → respond,
    /// so a suspended user or expired family is turned away before any write.
    async fn rotate_and_respond(
        &self,
        live: Session,
        via_grace: bool,
        request: &RefreshRequest,
    ) -> Result<TokenResponse> {
        assert!(
            !live.refresh_token_hash.expose().is_empty(),
            "rotate_and_respond: live generation must carry a hash"
        );

        // Expiry gate before any write: the family deadline is absolute.
        if live.expires_at < Utc::now() {
            return self
                .refuse_with_validation_failed(
                    EXPIRED_REFRESH_TOKEN_REASON,
                    Some(live.user_id.clone()),
                    request,
                )
                .await;
        }

        // User gates before any write.
        let user = match self.user_repo.get_user_by_id(&live.user_id).await? {
            Some(user) => user,
            None => {
                return self
                    .refuse_with_validation_failed(
                        MISSING_USER_REASON,
                        Some(live.user_id.clone()),
                        request,
                    )
                    .await
            }
        };
        if user.status != UserStatus::Active {
            // Suspension is a security outcome: emit on the mandatory channel so
            // no configured `emit_threshold` can drop it and a sink failure
            // follows `audit.durability` (the same shape the exchange flow's
            // terminal mapping produces).
            self.emit_security_event(
                SecurityEvent::PrincipalSuspended,
                AuditOutcome::Failure(AuditFailure::PrincipalSuspended),
                Some(user.id.clone()),
                None,
                request.client_addr.clone(),
                request.user_agent.clone(),
            )
            .await?;
            return Err(Error::UserSuspended { user_id: user.id });
        }

        let (raw_token, replacement) = self.mint_replacement(&live)?;

        // One atomic compare-and-swap conditioned on the live generation.
        // A `false` return means a concurrent redemption won: refuse without
        // revoking or alarming — the winner holds the replacement, and the
        // loser's retry lands on the grace path.
        let won = self
            .session_repo
            .rotate_refresh_token(live.refresh_token_hash.expose(), &replacement)
            .await?;
        if !won {
            return self
                .refuse_with_validation_failed(
                    UNKNOWN_REFRESH_TOKEN_REASON,
                    Some(user.id.clone()),
                    request,
                )
                .await;
        }

        // The sid names the family this rotation just served — the mint
        // postcondition guarantees it is well-formed here.
        let family_id_for_sid = &replacement.family_id;
        debug_assert!(is_valid_family_id(family_id_for_sid));
        let (access_token, expires_in) = self.build_access_token(&user, family_id_for_sid).await?;
        self.audit_successful_refresh(
            &user.id,
            &replacement.family_id,
            replacement.generation,
            via_grace,
            request,
        )
        .await?;

        Ok(TokenResponse {
            access_token,
            refresh_token: Some(crate::secret::Secret::new(raw_token)),
            token_type: "Bearer".to_string(),
            expires_in,
        })
    }

    /// The rotation-disabled flow: steps 1–2, 4, 5 and the signing/audit tail
    /// only. Nothing is minted, nothing is retired, and the response carries
    /// no refresh token — today's behaviour, preserved exactly. Leftover
    /// retirement classifications from a rotation-enabled period resolve as
    /// unknown: refused, but silent — no alarm, no revocation.
    async fn refresh_without_rotation(
        &self,
        resolution: RefreshResolution,
        request: &RefreshRequest,
    ) -> Result<TokenResponse> {
        let session = match resolution {
            RefreshResolution::Live(session) => session,
            RefreshResolution::Unknown
            | RefreshResolution::Superseded { .. }
            | RefreshResolution::Retired { .. } => {
                return self
                    .refuse_with_validation_failed(UNKNOWN_REFRESH_TOKEN_REASON, None, request)
                    .await;
            }
        };

        assert!(
            !session.refresh_token_hash.expose().is_empty(),
            "refresh_without_rotation: stored session must carry a hash"
        );

        if session.expires_at < Utc::now() {
            return self
                .refuse_with_validation_failed(
                    EXPIRED_REFRESH_TOKEN_REASON,
                    Some(session.user_id.clone()),
                    request,
                )
                .await;
        }

        let user = match self.user_repo.get_user_by_id(&session.user_id).await? {
            Some(user) => user,
            None => {
                return self
                    .refuse_with_validation_failed(
                        MISSING_USER_REASON,
                        Some(session.user_id.clone()),
                        request,
                    )
                    .await
            }
        };
        if user.status != UserStatus::Active {
            // Suspension is a security outcome: emit on the mandatory channel so
            // no configured `emit_threshold` can drop it and a sink failure
            // follows `audit.durability` (the same shape the exchange flow's
            // terminal mapping produces).
            self.emit_security_event(
                SecurityEvent::PrincipalSuspended,
                AuditOutcome::Failure(AuditFailure::PrincipalSuspended),
                Some(user.id.clone()),
                None,
                request.client_addr.clone(),
                request.user_agent.clone(),
            )
            .await?;
            return Err(Error::UserSuspended { user_id: user.id });
        }

        // The sid names the family the session already belongs to. A
        // pre-rotation legacy row carries the empty-string sentinel here:
        // minting a family would be rotation work, which this switch is off,
        // so the token is issued with an unusable sid and fails closed at
        // consumption time — the same posture as any hash-form sid.
        let family_id_for_sid = &session.family_id;
        debug_assert!(family_id_for_sid.is_empty() || is_valid_family_id(family_id_for_sid));
        let (access_token, expires_in) = self.build_access_token(&user, family_id_for_sid).await?;
        self.audit_successful_refresh(
            &user.id,
            &session.family_id,
            session.generation,
            false,
            request,
        )
        .await?;

        Ok(TokenResponse {
            access_token,
            refresh_token: None,
            token_type: "Bearer".to_string(),
            expires_in,
        })
    }

    /// Emit the successful-refresh audit: `TokenRefresh` at Info with
    /// `detail {family_id, generation, grace}`. No token hash appears here —
    /// family and generation are enough to correlate redemptions of one
    /// credential chain.
    async fn audit_successful_refresh(
        &self,
        user_id: &str,
        family_id: &str,
        generation: u32,
        via_grace: bool,
        request: &RefreshRequest,
    ) -> Result<()> {
        assert!(!user_id.is_empty(), "audit needs a real actor");
        assert!(
            family_id.is_empty() || is_valid_family_id(family_id),
            "audit detail carries the session family id, got {family_id:?}"
        );

        let detail = HashMap::from([
            ("family_id".to_string(), Value::from(family_id)),
            ("generation".to_string(), Value::from(generation)),
            ("grace".to_string(), Value::from(via_grace)),
        ]);
        // Refresh success is a security outcome: emit on the mandatory channel,
        // finally constructing the long-mapped
        // `AuthenticationSucceeded { kind: Refresh }` arm (`domain/audit.rs`).
        // Rendered `TokenRefresh` at Info with `{family_id, generation, grace}`
        // as before; only the channel changes.
        self.emit_security_event_with_detail(
            SecurityEvent::AuthenticationSucceeded {
                kind: AuthenticationKind::Refresh,
            },
            AuditOutcome::Success,
            Some(user_id.to_string()),
            None,
            request.client_addr.clone(),
            request.user_agent.clone(),
            detail,
        )
        .await
    }
}
