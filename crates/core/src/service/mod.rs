pub mod claims;
pub mod exchange;
pub mod refresh;
pub mod revoke;
pub mod user_admin;

use std::collections::HashMap;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::Utc;

use crate::config::AppConfig;
use crate::domain::{
    AccessTokenClaims, AuditEvent, AuditEventType, AuditOutcome, AuditSeverity, User,
};
use crate::error::{Error, Result};
use crate::ports::{
    AuditLog, IdentityProvider, KeyManager, SessionRepository, UserRepository, UserSync,
};

pub struct AppService {
    pub(crate) user_repo: Box<dyn UserRepository>,
    pub(crate) session_repo: Box<dyn SessionRepository>,
    pub(crate) keys: Box<dyn KeyManager>,
    pub(crate) audit: Box<dyn AuditLog>,
    pub(crate) user_sync: Box<dyn UserSync>,
    pub(crate) providers: HashMap<String, Box<dyn IdentityProvider>>,
    pub(crate) config: AppConfig,
}

impl AppService {
    pub fn new(
        user_repo: Box<dyn UserRepository>,
        session_repo: Box<dyn SessionRepository>,
        keys: Box<dyn KeyManager>,
        audit: Box<dyn AuditLog>,
        user_sync: Box<dyn UserSync>,
        providers: HashMap<String, Box<dyn IdentityProvider>>,
        config: AppConfig,
    ) -> Self {
        Self {
            user_repo,
            session_repo,
            keys,
            audit,
            user_sync,
            providers,
            config,
        }
    }

    /// Return the public key in JWK format for the JWKS endpoint.
    pub async fn public_jwk(&self) -> Result<serde_json::Value> {
        self.keys.public_jwk().await
    }

    /// Return the signing algorithm identifier (e.g. "EdDSA", "ES256").
    pub fn signing_algorithm(&self) -> &str {
        self.keys.algorithm()
    }

    /// Build and sign an access token JWT for the given user, bound to the
    /// session identified by `sid`.
    ///
    /// `sid` is the session's refresh-token hash: `/revoke` looks a presented
    /// access token up by exactly this value, so binding the hash at mint
    /// time is what makes "revoke this token" mean "end this one session"
    /// rather than "end every session of this subject".
    ///
    /// Returns `(jwt_string, expires_in_seconds)`.
    pub(crate) async fn build_access_token(&self, user: &User, sid: &str) -> Result<(String, u64)> {
        // Preconditions: a token without a subject authorizes nothing and one
        // without a session identifier could never be revoked through its own
        // `sid`, so either would mint an unusable credential — both are
        // programmer errors, not runtime conditions.
        assert!(
            !user.id.is_empty(),
            "build_access_token: user id must not be empty"
        );
        assert!(
            !sid.is_empty(),
            "build_access_token: session id must not be empty"
        );

        let now = Utc::now();
        let access_ttl_secs = parse_duration_secs(&self.config.token.access_token_ttl)?;

        let access_claims = AccessTokenClaims {
            sub: user.id.clone(),
            iss: self.config.server.issuer.clone(),
            aud: self.config.token.audience.clone().unwrap_or_default(),
            iat: now.timestamp() as u64,
            exp: (now.timestamp() as u64) + access_ttl_secs,
            sid: sid.to_string(),
            custom: claims::resolve_custom_claims(&self.config.token.custom_claims, user),
        };

        let claims_json = serde_json::to_vec(&access_claims).map_err(|e| Error::ConfigError {
            detail: format!("failed to serialize access token claims: {}", e),
        })?;

        let header = serde_json::json!({
            "alg": self.keys.algorithm(),
            // RFC 9068 §2.1 media type for a JWT access token; distinguishes
            // this artifact from every other JWT the same key might sign.
            "typ": ACCESS_TOKEN_TYP,
            "kid": self.keys.key_id()
        });
        let header_b64 = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&header).map_err(|e| {
            Error::ConfigError {
                detail: format!("failed to serialize JWT header: {}", e),
            }
        })?);
        let payload_b64 = URL_SAFE_NO_PAD.encode(&claims_json);
        let signing_input = format!("{}.{}", header_b64, payload_b64);
        let signature = self.keys.sign(signing_input.as_bytes()).await?;
        let sig_b64 = URL_SAFE_NO_PAD.encode(&signature);
        let access_token = format!("{}.{}", signing_input, sig_b64);

        Ok((access_token, access_ttl_secs))
    }

    /// The only path by which a claim of a service-minted JWT becomes
    /// readable. Validates a first-party access token in the source-spec
    /// order — shape, pinned header, signature, and only then typed claims —
    /// so no caller can observe a claim without proving everything before it.
    ///
    /// The `Err` carries one fixed reason constant, used solely as the audit
    /// `reason`; it never reaches the client.
    // TEMPORARY(03): unused until /revoke consumes it; removed in task 03.
    #[allow(dead_code)]
    pub(crate) async fn validate_access_token(
        &self,
        token: &str,
    ) -> std::result::Result<AccessTokenClaims, &'static str> {
        // One captured timestamp for every comparison in this validation, so
        // the exp/iat/nbf checks cannot disagree about "now".
        let now_secs = u64::try_from(Utc::now().timestamp()).unwrap_or(0);

        let (header_seg, payload_seg, signature_seg) = split_jws_segments(token)?;

        // The header is covered by the signature but is not self-authenticating:
        // pin it to what this service mints instead of reading it for direction.
        let header_bytes = URL_SAFE_NO_PAD
            .decode(header_seg)
            .map_err(|_| REASON_MALFORMED)?;
        pin_access_token_header(&header_bytes, self.keys.algorithm(), self.keys.key_id())?;

        // Verify over the original serialized bytes: the JWS signature covers
        // the segments exactly as received, and re-encoding could normalize
        // them into different bytes.
        let signing_input = format!("{}.{}", header_seg, payload_seg);
        let signature_bytes = URL_SAFE_NO_PAD
            .decode(signature_seg)
            .map_err(|_| REASON_MALFORMED)?;
        let verified = self
            .keys
            .verify(signing_input.as_bytes(), &signature_bytes)
            .await
            .map_err(|_| REASON_BAD_SIGNATURE)?;
        if !verified {
            return Err(REASON_BAD_SIGNATURE);
        }

        // Signature succeeded — only now may the payload be parsed and any
        // claim read.
        let payload_bytes = URL_SAFE_NO_PAD
            .decode(payload_seg)
            .map_err(|_| REASON_MALFORMED)?;
        let claims: AccessTokenClaims =
            serde_json::from_slice(&payload_bytes).map_err(|_| REASON_INVALID_CLAIMS)?;
        check_claims(&claims, &payload_bytes, &self.config, now_secs)?;

        // Postconditions the claim checks just established; re-asserting them
        // pairs the validation with a read-side check, so a future edit that
        // drops a check fails loudly here instead of leaking an unusable
        // credential to revocation.
        assert!(
            !claims.sub.trim().is_empty(),
            "validate_access_token: subject must be non-blank after check_claims"
        );
        assert!(
            !claims.sid.trim().is_empty(),
            "validate_access_token: session id must be non-blank after check_claims"
        );

        Ok(claims)
    }

    pub async fn emit_audit(&self, event: AuditEvent) -> Result<()> {
        // Pre-dispatch emit-threshold filter: events strictly less severe
        // than `[audit] emit_threshold` are dropped before any adapter ever
        // sees them, independently of the blocking-threshold decision below.
        let emit_threshold =
            parse_severity(&self.config.audit.emit_threshold).unwrap_or(AuditSeverity::Info);
        if event.severity as u8 > emit_threshold as u8 {
            return Ok(());
        }

        match self.audit.emit(&event).await {
            Ok(()) => Ok(()),
            Err(e) => {
                // Always emit via tracing as fallback (captured by Lambda, CloudWatch, etc.)
                let serialized =
                    serde_json::to_string(&event).unwrap_or_else(|_| format!("{:?}", event));

                if event.severity as u8 <= AuditSeverity::Error as u8 {
                    tracing::error!(audit_fallback = true, "{serialized}");
                } else {
                    tracing::info!(audit_fallback = true, "{serialized}");
                }

                // Parse blocking threshold from config
                let threshold = parse_severity(&self.config.audit.blocking_threshold)
                    .unwrap_or(AuditSeverity::Warning);

                if event.severity as u8 <= threshold as u8 {
                    // Severity meets blocking threshold — fail the operation
                    Err(e)
                } else {
                    tracing::warn!(error = %e, "audit provider down, event emitted to std stream");
                    Ok(())
                }
            }
        }
    }
}

/// Build an [`AuditEvent`]. `ip_address` and `user_agent` come from the
/// caller's client context (the `AuditContext` middleware at the HTTP edge);
/// the `AuditEvent` shape has no `device_id` field, so device identifiers are
/// recorded on the `Session` only, never on audit events.
#[allow(clippy::too_many_arguments)]
pub fn create_audit_event(
    event_type: AuditEventType,
    severity: AuditSeverity,
    outcome: AuditOutcome,
    actor: Option<String>,
    provider: Option<String>,
    ip_address: Option<String>,
    user_agent: Option<String>,
) -> AuditEvent {
    AuditEvent {
        id: ulid::Ulid::new().to_string(),
        timestamp: Utc::now(),
        severity,
        event_type,
        actor,
        provider,
        ip_address,
        user_agent,
        detail: HashMap::new(),
        outcome,
    }
}

pub fn parse_severity(s: &str) -> Option<AuditSeverity> {
    match s.trim().to_lowercase().as_str() {
        "emergency" => Some(AuditSeverity::Emergency),
        "alert" => Some(AuditSeverity::Alert),
        "critical" => Some(AuditSeverity::Critical),
        "error" => Some(AuditSeverity::Error),
        "warning" => Some(AuditSeverity::Warning),
        "notice" => Some(AuditSeverity::Notice),
        "info" => Some(AuditSeverity::Info),
        "debug" => Some(AuditSeverity::Debug),
        _ => None,
    }
}

/// Typed view of the JWT header this service mints. `alg`, `kid` and `typ`
/// are required struct fields, so a header missing any of them is a parse
/// failure rather than an optional read.
// TEMPORARY(03): unused until /revoke consumes the validator.
#[allow(dead_code)]
#[derive(serde::Deserialize)]
struct AccessTokenHeader {
    alg: String,
    kid: String,
    typ: String,
}

/// Typed view of the optional `nbf` payload claim. `nbf` is deliberately not
/// a field of [`AccessTokenClaims`] — the service never mints one — so it is
/// parsed separately, only after the required typed claims have succeeded.
// TEMPORARY(03): unused until /revoke consumes the validator.
#[allow(dead_code)]
#[derive(serde::Deserialize)]
struct NotBeforeClaim {
    nbf: Option<u64>,
}

/// Split a compact JWS into its three segments, rejecting any shape that is
/// not exactly three non-empty dot-separated parts.
// TEMPORARY(03): unused until /revoke consumes the validator.
#[allow(dead_code)]
fn split_jws_segments(token: &str) -> std::result::Result<(&str, &str, &str), &'static str> {
    let mut parts = token.split('.');
    if let (Some(header), Some(payload), Some(signature), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    {
        if !header.is_empty() && !payload.is_empty() && !signature.is_empty() {
            return Ok((header, payload, signature));
        }
    }
    Err(REASON_MALFORMED)
}

/// Decode and pin the access-token header to exactly what this service
/// mints: the key manager's algorithm and key id, and the RFC 9068
/// access-token media type.
// TEMPORARY(03): unused until /revoke consumes the validator.
#[allow(dead_code)]
fn pin_access_token_header(
    header_bytes: &[u8],
    algorithm: &str,
    key_id: &str,
) -> std::result::Result<(), &'static str> {
    // Preconditions on the pin values themselves: an unconfigured key
    // manager would make every pin comparison vacuous.
    assert!(
        !algorithm.is_empty(),
        "key manager must report an algorithm"
    );
    assert!(!key_id.is_empty(), "key manager must report a key id");

    let header: AccessTokenHeader =
        serde_json::from_slice(header_bytes).map_err(|_| REASON_WRONG_TYPE)?;

    if header.alg != algorithm || header.kid != key_id {
        return Err(REASON_WRONG_KEY);
    }
    if header.typ != ACCESS_TOKEN_TYP {
        return Err(REASON_WRONG_TYPE);
    }
    Ok(())
}

/// Validate the typed claims of a signature-verified payload: issuer,
/// audience, validity window (with [`CLOCK_SKEW_SECS`] leeway) and non-blank
/// identifiers, in the source-spec order.
///
/// Boundary semantics, made explicit: a token is expired when
/// `now > exp + CLOCK_SKEW_SECS` (so `now == exp + skew` is still valid),
/// issued in the future when `iat > now + CLOCK_SKEW_SECS`, and not yet
/// valid when `nbf > now + CLOCK_SKEW_SECS`. All comparisons are saturating
/// in `u64` so an absurd claim value can never wrap into acceptance.
// TEMPORARY(03): unused until /revoke consumes the validator.
#[allow(dead_code)]
fn check_claims(
    claims: &AccessTokenClaims,
    payload_bytes: &[u8],
    config: &AppConfig,
    now_secs: u64,
) -> std::result::Result<(), &'static str> {
    let expected_issuer = config.server.issuer.as_str();
    assert!(
        !expected_issuer.is_empty(),
        "server.issuer must be configured before tokens can be validated"
    );
    let skew = CLOCK_SKEW_SECS as u64;
    // The empty string is exactly what `build_access_token` stamps when
    // `token.audience` is unset, so mint and validate agree by construction.
    let expected_audience = config.token.audience.clone().unwrap_or_default();

    if claims.iss != expected_issuer {
        return Err(REASON_WRONG_ISSUER);
    }
    if claims.aud != expected_audience {
        return Err(REASON_WRONG_AUDIENCE);
    }

    // `nbf` is optional and parsed only after the typed required claims
    // above succeeded; a malformed (non-numeric) value is rejected.
    let not_before: NotBeforeClaim =
        serde_json::from_slice(payload_bytes).map_err(|_| REASON_INVALID_CLAIMS)?;

    if now_secs > claims.exp.saturating_add(skew) {
        return Err(REASON_EXPIRED);
    }
    if claims.iat > now_secs.saturating_add(skew) {
        return Err(REASON_FUTURE_ISSUED_AT);
    }
    if let Some(nbf) = not_before.nbf {
        if nbf > now_secs.saturating_add(skew) {
            return Err(REASON_NOT_YET_VALID);
        }
    }

    if claims.sub.trim().is_empty() {
        return Err(REASON_BLANK_SUBJECT);
    }
    if claims.sid.trim().is_empty() {
        return Err(REASON_BLANK_SESSION);
    }
    Ok(())
}

/// Seconds in one minute, for `m`-suffixed durations (`token.access_token_ttl` /
/// `refresh_token_ttl`).
const SECONDS_PER_MINUTE: u64 = 60;
/// Seconds in one hour, for `h`-suffixed durations.
const SECONDS_PER_HOUR: u64 = 60 * SECONDS_PER_MINUTE;
/// Seconds in one day, for `d`-suffixed durations.
const SECONDS_PER_DAY: u64 = 24 * SECONDS_PER_HOUR;

/// JWT header `typ` the service mints for access tokens (RFC 9068 §2.1) and
/// the validator pins to, so an access token cannot be confused with any
/// other JWT this key signs. Shared by minting and validation so the two
/// boundaries cannot drift apart.
pub(crate) const ACCESS_TOKEN_TYP: &str = "at+jwt";

/// Clock skew, in seconds, allowed on the `exp`/`iat`/`nbf` comparisons in
/// [`AppService::validate_access_token`]. Multi-node deployments and Lambda
/// cold starts drift; the bound is negligible against the shortest access
/// token TTL but keeps near-boundary tokens working across replicas.
// TEMPORARY(03): unused until /revoke consumes the validator.
#[allow(dead_code)]
pub(crate) const CLOCK_SKEW_SECS: i64 = 60;

/// A compact JWS is exactly three dot-separated segments: header, payload,
/// signature.
// TEMPORARY(03): unused until /revoke consumes the validator.
#[allow(dead_code)]
const JWT_SEGMENT_COUNT: usize = 3;

// Fixed rejection reasons for [`AppService::validate_access_token`]. Each is
// a constant literal suitable only for the audit `reason` field: none ever
// carries token bytes, decoded header/payload content, key-manager details,
// or a serde error, because the string is derived solely from which check
// failed, never from attacker-controlled data.
// TEMPORARY(03): reasons are consumed by check_claims, itself wired in 03.
#[allow(dead_code)]
const REASON_MALFORMED: &str = "malformed access token";
#[allow(dead_code)]
const REASON_WRONG_KEY: &str = "access token pinned to the wrong key";
#[allow(dead_code)]
const REASON_WRONG_TYPE: &str = "not an access token";
#[allow(dead_code)]
const REASON_BAD_SIGNATURE: &str = "invalid signature";
#[allow(dead_code)]
const REASON_INVALID_CLAIMS: &str = "malformed access token claims";
#[allow(dead_code)]
const REASON_WRONG_ISSUER: &str = "invalid issuer";
#[allow(dead_code)]
const REASON_WRONG_AUDIENCE: &str = "invalid audience";
#[allow(dead_code)]
const REASON_EXPIRED: &str = "token expired";
#[allow(dead_code)]
const REASON_FUTURE_ISSUED_AT: &str = "token issued in the future";
#[allow(dead_code)]
const REASON_NOT_YET_VALID: &str = "token not yet valid";
#[allow(dead_code)]
const REASON_BLANK_SUBJECT: &str = "blank subject";
#[allow(dead_code)]
const REASON_BLANK_SESSION: &str = "blank session identifier";

/// Parse a duration string like "15m", "1h", "30d" into seconds.
///
/// Never panics: the suffix is split on the last *character* (not byte), so a
/// multi-byte final character cannot land on a non-UTF-8 boundary, and unit
/// conversion uses checked multiplication so an overflowing value is reported
/// as a `ConfigError` rather than silently wrapping.
///
/// `pub` (not `pub(crate)`) so the `oidc-exchange` server crate can reuse the same parser for
/// `server.request_timeout` instead of duplicating duration-parsing logic for its
/// `TimeoutLayer` (`bootstrap::request_timeout_duration`).
pub fn parse_duration_secs(s: &str) -> Result<u64> {
    let s = s.trim();
    if s.is_empty() {
        return Err(Error::ConfigError {
            detail: "empty duration string".to_string(),
        });
    }

    // Split on the last *character*, not the last byte: `split_at` panics if
    // the index does not land on a char boundary, and `s.len() - 1` is only
    // safe when the final character is a single ASCII byte.
    let suffix_char = s
        .chars()
        .next_back()
        .expect("s is non-empty, checked above");
    let split_at = s.len() - suffix_char.len_utf8();
    let (num_str, suffix) = s.split_at(split_at);
    // Postcondition: the split partitions `s` exactly, and the suffix is the
    // single character we split off.
    assert_eq!(
        num_str.len() + suffix.len(),
        s.len(),
        "split_at must partition the whole string"
    );
    assert_eq!(
        suffix.chars().count(),
        1,
        "suffix must be exactly one character"
    );

    let value: u64 = num_str.parse().map_err(|_| Error::ConfigError {
        detail: format!("invalid duration number in {:?}: {}", s, num_str),
    })?;

    let secs = match suffix {
        "s" => Some(value),
        "m" => value.checked_mul(SECONDS_PER_MINUTE),
        "h" => value.checked_mul(SECONDS_PER_HOUR),
        "d" => value.checked_mul(SECONDS_PER_DAY),
        _ => {
            return Err(Error::ConfigError {
                detail: format!("unknown duration suffix in {:?}: {}", s, suffix),
            });
        }
    };

    secs.ok_or_else(|| Error::ConfigError {
        detail: format!("duration value overflows seconds when parsing {:?}", s),
    })
}

#[cfg(test)]
mod parse_duration_secs_tests {
    use super::*;

    #[test]
    fn parse_duration_secs_parses_seconds() {
        let secs = parse_duration_secs("45s").expect("valid duration");
        assert_eq!(secs, 45);
    }

    #[test]
    fn parse_duration_secs_parses_minutes() {
        let secs = parse_duration_secs("15m").expect("valid duration");
        assert_eq!(secs, 15 * SECONDS_PER_MINUTE);
        assert_eq!(secs, 900);
    }

    #[test]
    fn parse_duration_secs_parses_hours() {
        let secs = parse_duration_secs("1h").expect("valid duration");
        assert_eq!(secs, SECONDS_PER_HOUR);
        assert_eq!(secs, 3600);
    }

    #[test]
    fn parse_duration_secs_parses_days() {
        let secs = parse_duration_secs("30d").expect("valid duration");
        assert_eq!(secs, 30 * SECONDS_PER_DAY);
        assert_eq!(secs, 2_592_000);
    }

    #[test]
    fn parse_duration_secs_multi_byte_final_char_does_not_panic() {
        let err = parse_duration_secs("15€").expect_err("multi-byte suffix must be rejected");
        match err {
            Error::ConfigError { detail } => {
                assert!(
                    detail.contains("suffix"),
                    "detail should name the malformed suffix: {detail}"
                );
                assert!(
                    detail.contains("15€"),
                    "detail should name the offending input: {detail}"
                );
            }
            other => panic!("expected ConfigError, got {other:?}"),
        }
    }

    #[test]
    fn parse_duration_secs_overflowing_day_count_is_rejected() {
        let input = format!("{}d", u64::MAX);
        let err = parse_duration_secs(&input).expect_err("overflow must be rejected");
        match err {
            Error::ConfigError { detail } => {
                assert!(
                    detail.contains("overflow"),
                    "detail should call out the overflow: {detail}"
                );
            }
            other => panic!("expected ConfigError, got {other:?}"),
        }
    }

    #[test]
    fn parse_duration_secs_empty_string_is_rejected() {
        let err = parse_duration_secs("").expect_err("empty string must be rejected");
        match err {
            Error::ConfigError { detail } => {
                assert_eq!(detail, "empty duration string");
            }
            other => panic!("expected ConfigError, got {other:?}"),
        }
    }

    #[test]
    fn parse_duration_secs_unknown_suffix_is_rejected() {
        let err = parse_duration_secs("15x").expect_err("unknown suffix must be rejected");
        match err {
            Error::ConfigError { detail } => {
                assert!(
                    detail.contains("15x"),
                    "detail should name the offending input: {detail}"
                );
            }
            other => panic!("expected ConfigError, got {other:?}"),
        }
    }
}

/// Focused suite for [`AppService::validate_access_token`]. Lives beside the
/// validator because the method is `pub(crate)`: integration tests cannot
/// reach it, and every negative boundary below must drive the real check it
/// targets rather than a reimplementation.
///
/// The suite deliberately does not use `oidc-exchange-test-utils`: that crate
/// depends on this one, so its mocks would put two copies of
/// `oidc_exchange_core` into one build graph. Instead the ports the validator
/// never touches are panicking stubs — reaching one means the validator
/// performed I/O, which fails the test loudly — and signing uses a
/// deterministic toy key manager (`sign(p) == p`). Cryptographic strength is
/// the adapter's job; this suite exercises only the service's own validation
/// logic.
#[cfg(test)]
mod validate_access_token_tests {
    use std::collections::HashMap;

    use async_trait::async_trait;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    use chrono::Utc;
    use serde_json::{json, Value};

    use super::{
        AppService, ACCESS_TOKEN_TYP, CLOCK_SKEW_SECS, REASON_BAD_SIGNATURE, REASON_BLANK_SESSION,
        REASON_BLANK_SUBJECT, REASON_EXPIRED, REASON_FUTURE_ISSUED_AT, REASON_INVALID_CLAIMS,
        REASON_MALFORMED, REASON_NOT_YET_VALID, REASON_WRONG_AUDIENCE, REASON_WRONG_ISSUER,
        REASON_WRONG_KEY, REASON_WRONG_TYPE,
    };
    use crate::config::{AppConfig, ServerConfig, TokenConfig};
    use crate::domain::{
        AuditEvent, IdentityClaims, NewUser, ProviderTokens, Session, User, UserPatch,
    };
    use crate::error::Result;
    use crate::ports::{
        AuditLog, IdentityProvider, KeyManager, SessionRepository, UserRepository, UserSync,
    };

    /// Stub for the user/session ports: every method panics because the
    /// validator must decide validity without consulting any store.
    struct NeverTouchedStore;

    #[async_trait]
    impl UserRepository for NeverTouchedStore {
        async fn get_user_by_id(&self, _: &str) -> Result<Option<User>> {
            unreachable!("validate_access_token must not read the user repository")
        }
        async fn get_user_by_external_id(&self, _: &str, _: &str) -> Result<Option<User>> {
            unreachable!("validate_access_token must not read the user repository")
        }
        async fn create_user(&self, _: &NewUser) -> Result<User> {
            unreachable!("validate_access_token must not create users")
        }
        async fn update_user(&self, _: &str, _: &UserPatch) -> Result<User> {
            unreachable!("validate_access_token must not update users")
        }
        async fn delete_user(&self, _: &str) -> Result<()> {
            unreachable!("validate_access_token must not delete users")
        }
        async fn count_by_status(&self) -> Result<HashMap<String, u64>> {
            unreachable!("validate_access_token must not count users")
        }
        async fn list_users(&self, _: u64, _: u64) -> Result<Vec<User>> {
            unreachable!("validate_access_token must not list users")
        }
    }

    #[async_trait]
    impl SessionRepository for NeverTouchedStore {
        async fn store_refresh_token(&self, _: &Session) -> Result<()> {
            unreachable!("validate_access_token must not write sessions")
        }
        async fn get_session_by_refresh_token(&self, _: &str) -> Result<Option<Session>> {
            unreachable!(
                "validate_access_token must not consult the session store; \
                 signature and claims alone decide validity"
            )
        }
        async fn revoke_session(&self, _: &str) -> Result<()> {
            unreachable!("validate_access_token must never mutate session state")
        }
        async fn revoke_all_user_sessions(&self, _: &str) -> Result<()> {
            unreachable!("validate_access_token must never revoke all sessions")
        }
        async fn count_active_sessions(&self) -> Result<u64> {
            unreachable!("validate_access_token must not count sessions")
        }
        async fn cleanup_expired_sessions(&self) -> Result<u64> {
            unreachable!("validate_access_token must not reap sessions")
        }
    }

    /// Deterministic toy signer shared by the service under test and the
    /// token-minting helpers: the signature of a payload is the payload
    /// itself, so verification is an equality check.
    struct ToyKeyManager;

    #[async_trait]
    impl KeyManager for ToyKeyManager {
        async fn sign(&self, payload: &[u8]) -> Result<Vec<u8>> {
            Ok(payload.to_vec())
        }
        async fn verify(&self, payload: &[u8], signature: &[u8]) -> Result<bool> {
            Ok(signature == payload)
        }
        async fn public_jwk(&self) -> Result<serde_json::Value> {
            unreachable!("validate_access_token must not touch JWKS material")
        }
        fn algorithm(&self) -> &str {
            "EdDSA"
        }
        fn key_id(&self) -> &str {
            "test-key-1"
        }
    }

    struct NeverTouchedAuditLog;

    #[async_trait]
    impl AuditLog for NeverTouchedAuditLog {
        async fn emit(&self, _: &AuditEvent) -> Result<()> {
            unreachable!("validate_access_token must not audit")
        }
    }

    struct NeverTouchedUserSync;

    #[async_trait]
    impl UserSync for NeverTouchedUserSync {
        async fn notify_user_created(&self, _: &User) -> Result<()> {
            unreachable!("validate_access_token must not sync users")
        }
        async fn notify_user_updated(&self, _: &User, _: &[&str]) -> Result<()> {
            unreachable!("validate_access_token must not sync users")
        }
        async fn notify_user_deleted(&self, _: &str) -> Result<()> {
            unreachable!("validate_access_token must not sync users")
        }
    }

    struct NeverTouchedProvider;

    #[async_trait]
    impl IdentityProvider for NeverTouchedProvider {
        async fn exchange_code(&self, _: &str, _: &str) -> Result<ProviderTokens> {
            unreachable!("validate_access_token must not talk to providers")
        }
        async fn validate_id_token(&self, _: &str) -> Result<IdentityClaims> {
            unreachable!("validate_access_token must not validate provider tokens")
        }
        async fn revoke_token(&self, _: &str) -> Result<()> {
            unreachable!("validate_access_token must not revoke at providers")
        }
        fn provider_id(&self) -> &str {
            "never"
        }
    }

    fn test_config() -> AppConfig {
        AppConfig {
            server: ServerConfig {
                issuer: "https://auth.test.com".to_string(),
                ..Default::default()
            },
            token: TokenConfig {
                access_token_ttl: "15m".to_string(),
                refresh_token_ttl: "30d".to_string(),
                audience: Some("https://api.test.com".to_string()),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn test_service() -> AppService {
        AppService::new(
            Box::new(NeverTouchedStore),
            Box::new(NeverTouchedStore),
            Box::new(ToyKeyManager),
            Box::new(NeverTouchedAuditLog),
            Box::new(NeverTouchedUserSync),
            HashMap::from([("never".to_string(), Box::new(NeverTouchedProvider) as _)]),
            test_config(),
        )
    }

    /// Sign arbitrary header/payload JSON with the service's own key manager,
    /// exactly as minting does, so mutations below isolate one check each.
    async fn signed_token(svc: &AppService, header: &Value, payload: &Value) -> String {
        let header_b64 =
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(header).expect("header serializes"));
        let payload_b64 =
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(payload).expect("payload serializes"));
        let signing_input = format!("{}.{}", header_b64, payload_b64);
        let signature = svc
            .keys
            .sign(signing_input.as_bytes())
            .await
            .expect("signing succeeds");
        format!("{}.{}", signing_input, URL_SAFE_NO_PAD.encode(signature))
    }

    async fn valid_token(svc: &AppService, now: i64) -> String {
        signed_token(svc, &valid_header(), &valid_claims(now)).await
    }

    /// Canonical valid claims at `now`, matching `test_config`.
    fn valid_claims(now: i64) -> Value {
        json!({
            "sub": "usr_test",
            "iss": "https://auth.test.com",
            "aud": "https://api.test.com",
            "iat": now,
            "exp": now + 900,
            "sid": "a".repeat(64),
        })
    }

    /// Header exactly as the service mints it.
    fn valid_header() -> Value {
        json!({"alg": "EdDSA", "typ": ACCESS_TOKEN_TYP, "kid": "test-key-1"})
    }

    /// Flip the final character of the token while keeping every segment
    /// decodable, so the mutation reaches the verification check instead of
    /// dying at base64 decode.
    fn corrupt_signature(token: &str) -> String {
        let mut corrupted = token.to_string();
        let last = corrupted.pop().expect("token ends with signature data");
        corrupted.push(if last == 'A' { 'B' } else { 'A' });
        assert_ne!(corrupted, token, "the mutation must alter the token");
        corrupted
    }

    #[tokio::test]
    async fn accepts_a_correctly_signed_current_token() {
        let svc = test_service();
        let now = Utc::now().timestamp();

        let claims = svc
            .validate_access_token(&valid_token(&svc, now).await)
            .await
            .expect("a well-formed current token must validate");

        assert_eq!(claims.sub, "usr_test");
        assert_eq!(claims.sid, "a".repeat(64));
        assert_eq!(claims.iss, "https://auth.test.com");
        assert_eq!(claims.aud, "https://api.test.com");
        assert_eq!(claims.exp, (now + 900) as u64);
    }

    #[tokio::test]
    async fn rejects_non_three_segment_shapes_as_malformed() {
        let svc = test_service();

        let bad_shapes = ["", "only-one", "two.segments", "a.b.c.d", "a..c.d", "a.b."];
        for shape in bad_shapes {
            let err = svc
                .validate_access_token(shape)
                .await
                .expect_err("a malformed shape must be rejected");
            assert_eq!(
                err, REASON_MALFORMED,
                "shape {shape:?} must report malformed"
            );
        }

        // Non-base64url content fails decode cleanly rather than panicking.
        let err = svc
            .validate_access_token("!!not-base64!!.e30.e30")
            .await
            .expect_err("an undecodable header must be rejected");
        assert_eq!(err, REASON_MALFORMED);
    }

    #[tokio::test]
    async fn rejects_a_header_typ_other_than_at_jwt() {
        let svc = test_service();
        let now = Utc::now().timestamp();

        // Correctly signed generic-JWT header: the type pin fires.
        let token = signed_token(
            &svc,
            &json!({"alg": "EdDSA", "typ": "JWT", "kid": "test-key-1"}),
            &valid_claims(now),
        )
        .await;
        let err = svc
            .validate_access_token(&token)
            .await
            .expect_err("a generic JWT typ must not validate as an access token");
        assert_eq!(err, REASON_WRONG_TYPE);

        // A header missing `typ` entirely is a typed-header parse failure,
        // not an optional read.
        let token = signed_token(
            &svc,
            &json!({"alg": "EdDSA", "kid": "test-key-1"}),
            &valid_claims(now),
        )
        .await;
        let err = svc
            .validate_access_token(&token)
            .await
            .expect_err("a header without typ must be rejected");
        assert_eq!(err, REASON_WRONG_TYPE);
    }

    #[tokio::test]
    async fn rejects_headers_pinned_to_other_keys_or_algorithms() {
        let svc = test_service();
        let now = Utc::now().timestamp();

        let token = signed_token(
            &svc,
            &json!({"alg": "EdDSA", "typ": ACCESS_TOKEN_TYP, "kid": "other-key"}),
            &valid_claims(now),
        )
        .await;
        let err = svc
            .validate_access_token(&token)
            .await
            .expect_err("an unknown kid must be rejected");
        assert_eq!(err, REASON_WRONG_KEY);

        // Right key metadata shape, wrong algorithm family.
        let token = signed_token(
            &svc,
            &json!({"alg": "HS256", "typ": ACCESS_TOKEN_TYP, "kid": "test-key-1"}),
            &valid_claims(now),
        )
        .await;
        let err = svc
            .validate_access_token(&token)
            .await
            .expect_err("a foreign algorithm must be rejected");
        assert_eq!(err, REASON_WRONG_KEY);

        // A header without `kid` never parses as this service's typed
        // access-token header, so it lands in the same fixed bucket as the
        // other header-shape failures.
        let token = signed_token(
            &svc,
            &json!({"alg": "EdDSA", "typ": ACCESS_TOKEN_TYP}),
            &valid_claims(now),
        )
        .await;
        let err = svc
            .validate_access_token(&token)
            .await
            .expect_err("a header without kid must be rejected");
        assert_eq!(err, REASON_WRONG_TYPE);
    }

    #[tokio::test]
    async fn rejects_tampered_payloads_and_forged_signatures() {
        let svc = test_service();
        let now = Utc::now().timestamp();

        // A payload modified after signing no longer matches the signature
        // computed over the original bytes.
        let legit = signed_token(&svc, &valid_header(), &valid_claims(now)).await;
        let parts: Vec<&str> = legit.split('.').collect();
        let attacker_payload = URL_SAFE_NO_PAD.encode(
            json!({
                "sub": "usr_attacker",
                "iss": "https://auth.test.com",
                "aud": "https://api.test.com",
                "iat": now,
                "exp": now + 900,
                "sid": "b".repeat(64),
            })
            .to_string(),
        );
        let forged = format!("{}.{}.{}", parts[0], attacker_payload, parts[2]);
        let err = svc
            .validate_access_token(&forged)
            .await
            .expect_err("a tampered payload must fail signature verification");
        assert_eq!(err, REASON_BAD_SIGNATURE);

        // An intact-shape token whose signature was flipped also fails.
        let err = svc
            .validate_access_token(&corrupt_signature(&legit))
            .await
            .expect_err("a corrupted signature must fail verification");
        assert_eq!(err, REASON_BAD_SIGNATURE);
    }

    #[tokio::test]
    async fn rejects_missing_required_registered_claims() {
        let svc = test_service();
        let now = Utc::now().timestamp();

        let mut no_exp = valid_claims(now);
        no_exp
            .as_object_mut()
            .expect("claims are an object")
            .remove("exp")
            .expect("exp was present to remove");
        let token = signed_token(&svc, &valid_header(), &no_exp).await;
        let err = svc
            .validate_access_token(&token)
            .await
            .expect_err("missing exp must be a parse failure, not an omitted check");
        assert_eq!(err, REASON_INVALID_CLAIMS);

        let mut no_sid = valid_claims(now);
        no_sid
            .as_object_mut()
            .expect("claims are an object")
            .remove("sid")
            .expect("sid was present to remove");
        let token = signed_token(&svc, &valid_header(), &no_sid).await;
        let err = svc
            .validate_access_token(&token)
            .await
            .expect_err("missing sid must be a parse failure");
        assert_eq!(err, REASON_INVALID_CLAIMS);
    }

    #[tokio::test]
    async fn rejects_foreign_issuer_and_audience() {
        let svc = test_service();
        let now = Utc::now().timestamp();

        let mut foreign_iss = valid_claims(now);
        foreign_iss["iss"] = json!("https://evil.example.com");
        let token = signed_token(&svc, &valid_header(), &foreign_iss).await;
        let err = svc
            .validate_access_token(&token)
            .await
            .expect_err("a sibling deployment's issuer must be rejected");
        assert_eq!(err, REASON_WRONG_ISSUER);

        let mut foreign_aud = valid_claims(now);
        foreign_aud["aud"] = json!("https://other-api.example.com");
        let token = signed_token(&svc, &valid_header(), &foreign_aud).await;
        let err = svc
            .validate_access_token(&token)
            .await
            .expect_err("a mismatched audience must be rejected");
        assert_eq!(err, REASON_WRONG_AUDIENCE);
    }

    #[tokio::test]
    async fn rejects_expired_beyond_skew_but_accepts_exact_skew_edge() {
        let svc = test_service();
        let now = Utc::now().timestamp();

        let mut boundary = valid_claims(now);
        boundary["exp"] = json!((now - CLOCK_SKEW_SECS) as u64);
        let token = signed_token(&svc, &valid_header(), &boundary).await;
        svc.validate_access_token(&token)
            .await
            .expect("expiry exactly at the skew edge is still inside the window");

        let mut past_edge = valid_claims(now);
        past_edge["exp"] = json!((now - CLOCK_SKEW_SECS - 1) as u64);
        let token = signed_token(&svc, &valid_header(), &past_edge).await;
        let err = svc
            .validate_access_token(&token)
            .await
            .expect_err("one second past the skew edge the token is expired");
        assert_eq!(err, REASON_EXPIRED);
    }

    #[tokio::test]
    async fn rejects_future_iat_beyond_skew_but_accepts_exact_skew_edge() {
        let svc = test_service();
        let now = Utc::now().timestamp();

        let mut boundary = valid_claims(now);
        boundary["iat"] = json!((now + CLOCK_SKEW_SECS) as u64);
        let token = signed_token(&svc, &valid_header(), &boundary).await;
        svc.validate_access_token(&token)
            .await
            .expect("iat exactly at the skew edge is tolerated");

        let mut past_edge = valid_claims(now);
        past_edge["iat"] = json!((now + CLOCK_SKEW_SECS + 1) as u64);
        let token = signed_token(&svc, &valid_header(), &past_edge).await;
        let err = svc
            .validate_access_token(&token)
            .await
            .expect_err("iat one second past the skew edge is future-dated");
        assert_eq!(err, REASON_FUTURE_ISSUED_AT);
    }

    #[tokio::test]
    async fn optional_nbf_is_checked_with_the_same_skew_window() {
        let svc = test_service();
        let now = Utc::now().timestamp();

        let mut boundary = valid_claims(now);
        boundary["nbf"] = json!((now + CLOCK_SKEW_SECS) as u64);
        let token = signed_token(&svc, &valid_header(), &boundary).await;
        svc.validate_access_token(&token)
            .await
            .expect("nbf exactly at the skew edge is tolerated");

        let mut past_edge = valid_claims(now);
        past_edge["nbf"] = json!((now + CLOCK_SKEW_SECS + 1) as u64);
        let token = signed_token(&svc, &valid_header(), &past_edge).await;
        let err = svc
            .validate_access_token(&token)
            .await
            .expect_err("nbf one second past the skew edge means not yet valid");
        assert_eq!(err, REASON_NOT_YET_VALID);

        // A present-but-non-numeric nbf is rejected even though the typed
        // required claims above it succeeded.
        let mut garbage = valid_claims(now);
        garbage["nbf"] = json!("yesterday");
        let token = signed_token(&svc, &valid_header(), &garbage).await;
        let err = svc
            .validate_access_token(&token)
            .await
            .expect_err("a non-numeric nbf must be rejected");
        assert_eq!(err, REASON_INVALID_CLAIMS);

        // And a long-past nbf simply does not obstruct validity.
        let mut historical = valid_claims(now);
        historical["nbf"] = json!((now - 3600) as u64);
        let token = signed_token(&svc, &valid_header(), &historical).await;
        svc.validate_access_token(&token)
            .await
            .expect("a historical nbf does not invalidate the token");
    }

    #[tokio::test]
    async fn rejects_blank_subject_and_blank_session_identifiers() {
        let svc = test_service();
        let now = Utc::now().timestamp();

        let mut blank_sub = valid_claims(now);
        blank_sub["sub"] = json!("   ");
        let token = signed_token(&svc, &valid_header(), &blank_sub).await;
        let err = svc
            .validate_access_token(&token)
            .await
            .expect_err("a whitespace-only subject must be rejected");
        assert_eq!(err, REASON_BLANK_SUBJECT);

        let mut blank_sid = valid_claims(now);
        blank_sid["sid"] = json!("");
        let token = signed_token(&svc, &valid_header(), &blank_sid).await;
        let err = svc
            .validate_access_token(&token)
            .await
            .expect_err("an empty session identifier must be rejected");
        assert_eq!(err, REASON_BLANK_SESSION);
    }
}
