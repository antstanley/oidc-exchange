pub mod claims;
pub mod exchange;
pub mod refresh;
pub mod revoke;
pub mod user_admin;

use std::collections::HashMap;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::Utc;
use serde_json::Value;

use crate::config::AppConfig;
use crate::domain::{
    AccessTokenClaims, AuditEvent, AuditEventType, AuditOutcome, AuditSeverity, ClientAddr,
    SecurityEvent, User,
};
use crate::error::{Error, Result};
use crate::ports::{
    AuditLog, IdentityProvider, KeyManager, RateLimiter, SessionRepository, UserRepository,
    UserSync,
};

pub struct AppService {
    pub(crate) user_repo: Box<dyn UserRepository>,
    pub(crate) session_repo: Box<dyn SessionRepository>,
    pub(crate) keys: Box<dyn KeyManager>,
    pub(crate) audit: Box<dyn AuditLog>,
    pub(crate) user_sync: Box<dyn UserSync>,
    /// VENDORED SEAM (task 03): the shared rate-limit port from PR #24. On
    /// this branch only the admin plane's `OperatorAuth` budget flows through
    /// it; the exchange plane's budgets arrive with #24's merge.
    pub(crate) rate_limiter: Box<dyn RateLimiter>,
    pub(crate) providers: HashMap<String, Box<dyn IdentityProvider>>,
    pub(crate) config: AppConfig,
}

impl AppService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        user_repo: Box<dyn UserRepository>,
        session_repo: Box<dyn SessionRepository>,
        keys: Box<dyn KeyManager>,
        audit: Box<dyn AuditLog>,
        user_sync: Box<dyn UserSync>,
        rate_limiter: Box<dyn RateLimiter>,
        providers: HashMap<String, Box<dyn IdentityProvider>>,
        config: AppConfig,
    ) -> Self {
        Self {
            user_repo,
            session_repo,
            keys,
            audit,
            user_sync,
            rate_limiter,
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

    /// The shared [`RateLimiter`] port instance.
    ///
    /// Exposed so the server's operator-auth layer can consult the same
    /// budget this service was built with — the throttle state must be one
    /// process-wide instance, never a per-consumer copy.
    pub fn rate_limiter(&self) -> &dyn RateLimiter {
        self.rate_limiter.as_ref()
    }

    /// Build and sign an access token JWT for the given user.
    ///
    /// Returns `(jwt_string, expires_in_seconds)`.
    pub(crate) async fn build_access_token(&self, user: &User) -> Result<(String, u64)> {
        let now = Utc::now();
        let access_ttl_secs = parse_duration_secs(&self.config.token.access_token_ttl)?;

        let access_claims = AccessTokenClaims {
            sub: user.id.clone(),
            iss: self.config.server.issuer.clone(),
            aud: self.config.token.audience.clone().unwrap_or_default(),
            iat: now.timestamp() as u64,
            exp: (now.timestamp() as u64) + access_ttl_secs,
            custom: claims::resolve_custom_claims(&self.config.token.custom_claims, user),
        };

        let claims_json = serde_json::to_vec(&access_claims).map_err(|e| Error::ConfigError {
            detail: format!("failed to serialize access token claims: {}", e),
        })?;

        let header = serde_json::json!({
            "alg": self.keys.algorithm(),
            "typ": "JWT",
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

    /// Emit a [`SecurityEvent`] through the mandatory audit channel.
    ///
    /// VENDORED SEAM (task 03): the mandatory channel itself, modelled on PR
    /// #24's `emit_security_event`. "Mandatory" here means the
    /// `emit_threshold` filter never applies — a guessing campaign against
    /// the privilege-assignment primitive must be recorded even in a
    /// deployment that filters routine events down to `notice`. Sink failure
    /// keeps the existing blocking-threshold contract (fail the request when
    /// the event's severity meets `[audit] blocking_threshold`, else log and
    /// continue); #24's durability counters and `audit.durability` config are
    /// that PR's own scope and are not vendored.
    ///
    /// The event's classification (type, severity) is fixed by the closed
    /// [`SecurityEvent`] enum; the caller supplies only context. The failure
    /// reason travels in `outcome` as one of the fixed
    /// [`crate::domain::security_failure_reasons`] strings.
    pub async fn emit_security_event(
        &self,
        event: SecurityEvent,
        outcome: AuditOutcome,
        actor: Option<String>,
        client_addr: ClientAddr,
        detail: HashMap<String, Value>,
    ) -> Result<()> {
        // Admin-plane security events carry the peer address, never a
        // forwarded or asserted one, and never the presented credential.
        let audit_event = AuditEvent {
            id: ulid::Ulid::new().to_string(),
            timestamp: Utc::now(),
            severity: event.severity(),
            event_type: event.event_type(),
            actor,
            // The security event is about the *caller's* attempt, not about
            // any user; the operator field stays None because authentication
            // did not (or in the throttle's case, could not) conclude.
            operator: None,
            provider: None,
            ip_address: client_addr.audit_address(),
            user_agent: None,
            detail,
            outcome,
        };
        // Postcondition: a security event never carries an operator identity —
        // it records attempts, not completed authentications.
        assert!(
            audit_event.operator.is_none(),
            "security events record authentication attempts and must never carry a principal"
        );
        assert_eq!(
            audit_event.severity,
            event.severity(),
            "security-event severity must come from the closed enum, never from the caller"
        );

        match self.audit.emit(&audit_event).await {
            Ok(()) => Ok(()),
            Err(e) => {
                let serialized = serde_json::to_string(&audit_event)
                    .unwrap_or_else(|_| format!("{:?}", audit_event));
                if audit_event.severity as u8 <= AuditSeverity::Error as u8 {
                    tracing::error!(audit_fallback = true, "{serialized}");
                } else {
                    tracing::warn!(audit_fallback = true, "{serialized}");
                }

                let threshold = parse_severity(&self.config.audit.blocking_threshold)
                    .unwrap_or(AuditSeverity::Warning);
                if audit_event.severity as u8 <= threshold as u8 {
                    Err(e)
                } else {
                    tracing::warn!(error = %e, "mandatory audit provider down, event emitted to std stream");
                    Ok(())
                }
            }
        }
    }
}

/// Build an [`AuditEvent`] with no operator attribution.
///
/// `ip_address` and `user_agent` come from the caller's client context (the
/// `AuditContext` middleware at the HTTP edge); the `AuditEvent` shape has no
/// `device_id` field, so device identifiers are recorded on the `Session`
/// only, never on audit events. The `operator` field starts as `None`: this
/// constructor serves every exchange-plane event, where there is no operator,
/// and admin flows stamp the principal onto the returned event explicitly so
/// attribution is always a deliberate act, never a default.
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
        operator: None,
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

/// Seconds in one minute, for `m`-suffixed durations (`token.access_token_ttl` /
/// `refresh_token_ttl`).
const SECONDS_PER_MINUTE: u64 = 60;
/// Seconds in one hour, for `h`-suffixed durations.
const SECONDS_PER_HOUR: u64 = 60 * SECONDS_PER_MINUTE;
/// Seconds in one day, for `d`-suffixed durations.
const SECONDS_PER_DAY: u64 = 24 * SECONDS_PER_HOUR;

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
