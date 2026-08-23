//! Server-layer operator authentication for `/internal/*`.
//!
//! The [`OperatorAuthenticator`] trait and its three implementations live
//! here — deliberately **not** as a core port: they authenticate HTTP request
//! parts, which `crates/core` does not model (`02-ports-and-adapters.md` →
//! Port traits). The core owns [`OperatorPrincipal`]; the server owns the
//! authentication of one.
//!
//! Mechanisms are tried in configured order and the first success wins:
//!
//! | Mechanism | Credential | Principal id |
//! |---|---|---|
//! | `shared_secret` | `Authorization: Bearer <secret>`, constant-time compare | the reserved `unattributed` |
//! | `operator_token` | `Authorization: Bearer <jwt>` verified against this service's own key manager | the token's `sub` |
//! | `mtls` | client-certificate subject from the terminating proxy's header | the certificate subject |
//!
//! Every rejection is reported with a fixed [`OperatorAuthFailureReason`] —
//! never free-form text, never any part of the presented credential.

use std::collections::HashMap;

use axum::http::HeaderMap;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde_json::Value;
use subtle::ConstantTimeEq;

use oidc_exchange_core::domain::{
    OperatorAuthFailureReason, OperatorAuthMechanism, OperatorPrincipal, UNATTRIBUTED_OPERATOR_ID,
};
use oidc_exchange_core::ports::KeyManager;

/// Upper bound, in bytes, on an accepted mTLS subject header value. A real
/// distinguished name is far below this; anything larger is a hostile or
/// misconfigured proxy assertion, not an identity.
pub const MAX_MTLS_SUBJECT_BYTES: usize = 4096;

/// Clock-skew leeway, in seconds, applied when checking an operator token's
/// expiry: a token within this window of expiring is still accepted. Signing
/// and verifying hosts disagree by fractions of a second; a zero-leeway check
/// would reject valid credentials at phase boundaries.
pub const OPERATOR_TOKEN_CLOCK_SKEW_LEEWAY_SECS: u64 = 30;

/// Upper bound on the remaining lifetime of an accepted operator token, in
/// seconds (24 hours). An operator credential is a short-lived service token;
/// an `exp` further out than this bound signals a minting misconfiguration,
/// which validation rejects rather than silently honouring.
pub const MAX_OPERATOR_TOKEN_TTL_SECS: u64 = 24 * 60 * 60;

/// What the authenticators may inspect: the bearer token (if any was
/// presented) and the request's headers. Nothing else about the request is
/// relevant to authentication, and no presented credential ever leaves this
/// type — rejections carry only fixed reasons.
#[derive(Debug, Clone)]
pub struct AuthInput<'a> {
    pub bearer: Option<&'a str>,
    pub headers: &'a HeaderMap,
}

impl<'a> AuthInput<'a> {
    /// Collect the authentication-relevant parts of a request.
    pub fn from_parts(bearer: Option<&'a str>, headers: &'a HeaderMap) -> Self {
        Self { bearer, headers }
    }
}

/// One mechanism's way of turning a presented credential into a principal.
///
/// Implementations are constructed once at startup from validated config; the
/// layer then drives them in configuration order.
#[async_trait::async_trait]
pub trait OperatorAuthenticator: Send + Sync {
    /// The mechanism this authenticator implements.
    fn mechanism(&self) -> OperatorAuthMechanism;

    /// Authenticate the presented credential.
    ///
    /// `Ok(principal)` admits the request; `Err(reason)` rejects it with a
    /// closed-vocabulary reason. Infrastructure failures inside verification
    /// (e.g. the key manager erroring) also surface here as
    /// [`OperatorAuthFailureReason::InvalidCredential`] so a failing backend
    /// cannot be distinguished from a bad credential by an unauthenticated
    /// caller — the difference is logged, not leaked through the wire.
    async fn authenticate(
        &self,
        input: &AuthInput<'_>,
    ) -> Result<OperatorPrincipal, OperatorAuthFailureReason>;
}

/// The shared-secret compatibility mechanism.
///
/// Proves possession of one configured string via constant-time comparison
/// and identifies nobody: every success yields the reserved
/// [`UNATTRIBUTED_OPERATOR_ID`] principal.
pub struct SharedSecretAuthenticator {
    expected: String,
}

impl SharedSecretAuthenticator {
    pub fn new(expected: String) -> Self {
        Self { expected }
    }
}

impl std::fmt::Debug for SharedSecretAuthenticator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The secret is the plane's entire authentication under its mechanism;
        // neither value nor length may leak through Debug output into logs.
        f.debug_struct("SharedSecretAuthenticator")
            .field("expected", &"<redacted>")
            .finish()
    }
}

#[async_trait::async_trait]
impl OperatorAuthenticator for SharedSecretAuthenticator {
    fn mechanism(&self) -> OperatorAuthMechanism {
        OperatorAuthMechanism::SharedSecret
    }

    async fn authenticate(
        &self,
        input: &AuthInput<'_>,
    ) -> Result<OperatorPrincipal, OperatorAuthFailureReason> {
        let Some(presented) = input.bearer else {
            return Err(OperatorAuthFailureReason::MissingCredential);
        };
        // Constant-time comparison with explicit length-branch handling, so
        // no timing oracle survives the migration to named mechanisms.
        if constant_time_eq(presented.as_bytes(), self.expected.as_bytes()) {
            Ok(OperatorPrincipal::unattributed())
        } else {
            Err(OperatorAuthFailureReason::InvalidCredential)
        }
    }
}

/// The named-principal operator-token mechanism.
///
/// Verifies a JWT against this service's own [`KeyManager`] — the same trust
/// anchor that signs access tokens — and requires `iss`, `aud`, an unexpired
/// window, and the configured claim/value before accepting the token's `sub`
/// as the principal id.
pub struct OperatorTokenAuthenticator {
    keys: Box<dyn KeyManager>,
    issuer: String,
    audience: String,
    required_claim: String,
    required_value: String,
}

impl OperatorTokenAuthenticator {
    pub fn new(
        keys: Box<dyn KeyManager>,
        issuer: String,
        audience: String,
        required_claim: String,
        required_value: String,
    ) -> Self {
        assert!(
            !issuer.trim().is_empty(),
            "operator-token verification requires a non-empty issuer"
        );
        assert!(
            !audience.trim().is_empty(),
            "operator-token verification requires a non-empty audience"
        );
        assert!(
            !required_claim.trim().is_empty(),
            "operator-token verification requires a non-empty required claim name"
        );
        Self {
            keys,
            issuer,
            audience,
            required_claim,
            required_value,
        }
    }

    /// Decode and structurally validate the token envelope, returning the
    /// payload claims and the header algorithm.
    fn decode(token: &str) -> Option<(String, serde_json::Map<String, Value>)> {
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            return None;
        }

        let header_bytes = URL_SAFE_NO_PAD.decode(parts[0]).ok()?;
        let header: Value = serde_json::from_slice(&header_bytes).ok()?;
        let alg = header.get("alg")?.as_str()?.to_string();

        let payload_bytes = URL_SAFE_NO_PAD.decode(parts[1]).ok()?;
        let payload: Value = serde_json::from_slice(&payload_bytes).ok()?;
        let claims = payload.as_object().cloned()?;

        let _signature = URL_SAFE_NO_PAD.decode(parts[2]).ok()?;

        Some((alg, claims))
    }

    /// Verify the signature over the signing input with this service's key
    /// manager. Returns `None` on malformed signature or infrastructure
    /// failure — both are simply "not verified".
    async fn verify_signature(&self, signing_input: &str, signature: &[u8]) -> Option<()> {
        if self.keys.verify(signing_input.as_bytes(), signature).await.ok()? {
            Some(())
        } else {
            None
        }
    }
}

impl std::fmt::Debug for OperatorTokenAuthenticator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // No key material or expected claim *value* is printable state; only
        // the non-secret policy fields are shown.
        f.debug_struct("OperatorTokenAuthenticator")
            .field("issuer", &self.issuer)
            .field("audience", &self.audience)
            .field("required_claim", &self.required_claim)
            .finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl OperatorAuthenticator for OperatorTokenAuthenticator {
    fn mechanism(&self) -> OperatorAuthMechanism {
        OperatorAuthMechanism::OperatorToken
    }

    async fn authenticate(
        &self,
        input: &AuthInput<'_>,
    ) -> Result<OperatorPrincipal, OperatorAuthFailureReason> {
        let invalid = OperatorAuthFailureReason::InvalidCredential;

        let Some(token) = input.bearer else {
            return Err(OperatorAuthFailureReason::MissingCredential);
        };

        let (alg, claims) = Self::decode(token).ok_or(invalid)?;

        // Algorithm pinning: a token is only ever signed by this service's own
        // key manager and algorithm, so cross-algorithm confusion ("none",
        // symmetric downgrade) fails before any cryptographic call.
        if alg != self.keys.algorithm() {
            tracing::warn!(
                presented_alg = %alg,
                expected_alg = %self.keys.algorithm(),
                "operator token rejected: algorithm mismatch"
            );
            return Err(invalid());
        }

        let parts: Vec<&str> = token.split('.').collect();
        debug_assert_eq!(parts.len(), 3, "decode() already enforced the three-part shape");
        let signature = URL_SAFE_NO_PAD
            .decode(parts[2])
            .map_err(|_| invalid)?;
        let signing_input = format!("{}.{}", parts[0], parts[1]);
        if self.verify_signature(&signing_input, &signature).await.is_none() {
            return Err(invalid());
        }

        // iss must equal this service's issuer exactly.
        if claims.get("iss").and_then(Value::as_str) != Some(self.issuer.as_str()) {
            return Err(invalid());
        }

        // aud must contain the internal-API audience (string-or-array, per
        // RFC 7519 §4.1.3).
        let aud_matches = match claims.get("aud") {
            Some(Value::String(aud)) => aud == self.audience,
            Some(Value::Array(auds)) => auds
                .iter()
                .filter_map(Value::as_str)
                .any(|aud| aud == self.audience),
            _ => false,
        };
        if !aud_matches {
            return Err(invalid());
        }

        // Unexpired window, with bounded skew leeway and a hard ceiling on
        // accepted remaining lifetime.
        let now = now_unix_secs();
        let exp = claims.get("exp").and_then(Value::as_u64).ok_or(invalid)?;
        if now >= exp.saturating_add(OPERATOR_TOKEN_CLOCK_SKEW_LEEWAY_SECS) {
            return Err(invalid()); // expired
        }
        if exp > now + MAX_OPERATOR_TOKEN_TTL_SECS + OPERATOR_TOKEN_CLOCK_SKEW_LEEWAY_SECS {
            tracing::warn!(exp = exp, "operator token rejected: implausibly long lifetime");
            return Err(invalid());
        }

        // The configured claim must be present with exactly the required value.
        if claims.get(&self.required_claim).and_then(Value::as_str)
            != Some(self.required_value.as_str())
        {
            return Err(invalid());
        }

        // Only a non-empty subject identifies anyone.
        let subject = claims
            .get("sub")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or(invalid)?;

        let principal = OperatorPrincipal {
            id: subject.to_string(),
            mechanism: OperatorAuthMechanism::OperatorToken,
        };
        principal.assert_invariants();

        Ok(principal)
    }
}

/// Current unix time in seconds. Wrapped so the clock source has exactly one
/// definition point in this module.
fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The proxy-asserted mTLS-subject mechanism.
///
/// Reads the client-certificate subject from the header the TLS-terminating
/// proxy sets. This is trustworthy only because the layer runs exclusively on
/// the admin listener, whose default host is loopback and behind no untrusted
/// proxy — the public router never mounts it.
pub struct MtlsSubjectAuthenticator {
    subject_header: String,
}

impl MtlsSubjectAuthenticator {
    pub fn new(subject_header: String) -> Self {
        assert!(
            !subject_header.trim().is_empty(),
            "the mtls mechanism requires a non-empty subject header name"
        );
        Self {
            // HTTP/2 lowercases header names; normalize once at construction
            // so a mixed-case config entry cannot miss every request.
            subject_header: subject_header.to_ascii_lowercase(),
        }
    }
}

impl std::fmt::Debug for MtlsSubjectAuthenticator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MtlsSubjectAuthenticator")
            .field("subject_header", &self.subject_header)
            .finish()
    }
}

#[async_trait::async_trait]
impl OperatorAuthenticator for MtlsSubjectAuthenticator {
    fn mechanism(&self) -> OperatorAuthMechanism {
        OperatorAuthMechanism::MutualTls
    }

    async fn authenticate(
        &self,
        input: &AuthInput<'_>,
    ) -> Result<OperatorPrincipal, OperatorAuthFailureReason> {
        let Some(value) = input.headers.get(&self.subject_header) else {
            return Err(OperatorAuthFailureReason::MissingCredential);
        };

        let Ok(subject) = value.to_str() else {
            // A non-ASCII assertion is a malformed identity, not a name.
            return Err(OperatorAuthFailureReason::InvalidCredential);
        };
        let subject = subject.trim();
        if subject.is_empty() {
            return Err(OperatorAuthFailureReason::InvalidCredential);
        }
        if subject.len() > MAX_MTLS_SUBJECT_BYTES {
            tracing::warn!(
                len = subject.len(),
                "mtls subject header rejected: exceeds the declared bound"
            );
            return Err(OperatorAuthFailureReason::InvalidCredential);
        }

        let principal = OperatorPrincipal {
            id: subject.to_string(),
            mechanism: OperatorAuthMechanism::MutualTls,
        };
        principal.assert_invariants();
        Ok(principal)
    }
}

/// Constant-time byte comparison using the `subtle` crate, with explicit
/// length-branch handling so comparing values of different lengths still
/// performs a comparison-shaped amount of work.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        // Still do a comparison to avoid timing leak on length,
        // but we know it will be false.
        let _ = a.ct_eq(&vec![0u8; a.len()]);
        return false;
    }
    a.ct_eq(b).into()
}

/// The configured chain of mechanisms, tried in order.
pub struct OperatorAuthGate {
    authenticators: Vec<Box<dyn OperatorAuthenticator>>,
}

impl OperatorAuthGate {
    /// Assemble a gate from already-validated authenticators.
    pub fn new(authenticators: Vec<Box<dyn OperatorAuthenticator>>) -> Self {
        assert!(
            !authenticators.is_empty(),
            "an operator-auth gate needs at least one mechanism"
        );
        Self { authenticators }
    }

    /// Try each configured mechanism in order; first success wins.
    ///
    /// When everything fails, the returned reason follows the spec's
    /// precedence: `invalid_credential` beats `missing_credential` (something
    /// was presented and rejected outranks nothing being presented), and
    /// `not_configured` is reserved for a served plane that somehow reached
    /// here with no usable mechanism.
    pub async fn authenticate(
        &self,
        input: &AuthInput<'_>,
    ) -> Result<OperatorPrincipal, OperatorAuthFailureReason> {
        let mut saw_presented_credential = false;
        for authenticator in &self.authenticators {
            match authenticator.authenticate(input).await {
                Ok(principal) => {
                    principal.assert_invariants();
                    return Ok(principal);
                }
                Err(OperatorAuthFailureReason::MissingCredential) => continue,
                Err(OperatorAuthFailureReason::InvalidCredential) => {
                    saw_presented_credential = true;
                }
                Err(OperatorAuthFailureReason::NotConfigured) => {}
            }
        }

        Err(if saw_presented_credential {
            OperatorAuthFailureReason::InvalidCredential
        } else {
            OperatorAuthFailureReason::NotConfigured
        })
    }
}

impl std::fmt::Debug for OperatorAuthGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Only the mechanism list is printable; authenticator internals may
        // hold secrets even though their own Debug impls redact them.
        let mechanisms: Vec<OperatorAuthMechanism> =
            self.authenticators.iter().map(|a| a.mechanism()).collect();
        f.debug_struct("OperatorAuthGate")
            .field("mechanisms", &mechanisms)
            .finish()
    }
}

/// Build a security-event detail map naming the route an attempt hit.
/// Deliberately tiny: the route and nothing else — the presented credential
/// must never enter the audit stream.
pub fn auth_event_detail(route: &str) -> HashMap<String, Value> {
    let mut detail = HashMap::new();
    detail.insert(
        "route".to_string(),
        Value::String(route.to_string()),
    );
    detail
}
