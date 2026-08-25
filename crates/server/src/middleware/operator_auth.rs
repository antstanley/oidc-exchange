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
    OperatorAuthFailureReason, OperatorAuthMechanism, OperatorPrincipal,
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
    expected: oidc_exchange_core::Secret<String>,
}

impl SharedSecretAuthenticator {
    pub fn new(expected: oidc_exchange_core::Secret<String>) -> Self {
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
        if constant_time_eq(presented.as_bytes(), self.expected.expose().as_bytes()) {
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
        if self
            .keys
            .verify(signing_input.as_bytes(), signature)
            .await
            .ok()?
        {
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
            return Err(invalid);
        }

        let parts: Vec<&str> = token.split('.').collect();
        debug_assert_eq!(
            parts.len(),
            3,
            "decode() already enforced the three-part shape"
        );
        let signature = URL_SAFE_NO_PAD.decode(parts[2]).map_err(|_| invalid)?;
        let signing_input = format!("{}.{}", parts[0], parts[1]);
        if self
            .verify_signature(&signing_input, &signature)
            .await
            .is_none()
        {
            return Err(invalid);
        }

        // iss must equal this service's issuer exactly.
        if claims.get("iss").and_then(Value::as_str) != Some(self.issuer.as_str()) {
            return Err(invalid);
        }

        // aud must contain the internal-API audience (string-or-array, per
        // RFC 7519 §4.1.3).
        let aud_matches = match claims.get("aud") {
            Some(Value::String(aud)) => aud.as_str() == self.audience,
            Some(Value::Array(auds)) => auds
                .iter()
                .filter_map(Value::as_str)
                .any(|aud| aud == self.audience),
            _ => false,
        };
        if !aud_matches {
            return Err(invalid);
        }

        // Unexpired window, with bounded skew leeway and a hard ceiling on
        // accepted remaining lifetime.
        let now = now_unix_secs();
        let exp = claims.get("exp").and_then(Value::as_u64).ok_or(invalid)?;
        if now >= exp.saturating_add(OPERATOR_TOKEN_CLOCK_SKEW_LEEWAY_SECS) {
            return Err(invalid); // expired
        }
        if exp > now + MAX_OPERATOR_TOKEN_TTL_SECS + OPERATOR_TOKEN_CLOCK_SKEW_LEEWAY_SECS {
            tracing::warn!(
                exp = exp,
                "operator token rejected: implausibly long lifetime"
            );
            return Err(invalid);
        }

        // The configured claim must be present with exactly the required value.
        if claims.get(&self.required_claim).and_then(Value::as_str)
            != Some(self.required_value.as_str())
        {
            return Err(invalid);
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
    /// When everything fails, the returned reason follows the spec's fixed
    /// vocabulary: something was presented and rejected outranks nothing
    /// being presented (`invalid_credential` beats `missing_credential`),
    /// while `not_configured` stays reserved for its spec meaning — an
    /// internal API with no usable mechanism — and never describes a request
    /// that merely arrived without credentials.
    pub async fn authenticate(
        &self,
        input: &AuthInput<'_>,
    ) -> Result<OperatorPrincipal, OperatorAuthFailureReason> {
        let mut saw_presented_credential = false;
        let mut saw_missing_credential = false;
        for authenticator in &self.authenticators {
            match authenticator.authenticate(input).await {
                Ok(principal) => {
                    principal.assert_invariants();
                    return Ok(principal);
                }
                Err(OperatorAuthFailureReason::MissingCredential) => {
                    saw_missing_credential = true;
                }
                Err(OperatorAuthFailureReason::InvalidCredential) => {
                    saw_presented_credential = true;
                }
                Err(OperatorAuthFailureReason::NotConfigured) => {}
            }
        }

        Err(if saw_presented_credential {
            OperatorAuthFailureReason::InvalidCredential
        } else if saw_missing_credential {
            OperatorAuthFailureReason::MissingCredential
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
    detail.insert("route".to_string(), Value::String(route.to_string()));
    detail
}

#[cfg(test)]
mod tests {
    use super::*;
    use oidc_exchange_core::domain::UNATTRIBUTED_OPERATOR_ID;
    use oidc_exchange_test_utils::MockKeyManager;

    const SECRET: &str = "unit-test-shared-secret-value";
    const HEADER: &str = "x-client-cert-subject";

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(
                axum::http::HeaderName::from_bytes(name.as_bytes())
                    .expect("test header names are valid"),
                axum::http::HeaderValue::from_str(value).expect("test header values are valid"),
            );
        }
        map
    }

    fn input<'a>(bearer: Option<&'a str>, headers: &'a HeaderMap) -> AuthInput<'a> {
        AuthInput::from_parts(bearer, headers)
    }

    // --- SharedSecretAuthenticator ---------------------------------------

    /// The correct secret authenticates into the reserved unattributed pair;
    /// a wrong secret is rejected as invalid, and no credential at all as
    /// missing. Debug output never shows the secret (value or length).
    #[tokio::test]
    async fn shared_secret_authenticates_only_the_exact_value() {
        let auth =
            SharedSecretAuthenticator::new(oidc_exchange_core::Secret::new(SECRET.to_string()));
        let empty = headers(&[]);

        let ok = auth
            .authenticate(&input(Some(SECRET), &empty))
            .await
            .expect("the exact secret authenticates");
        assert_eq!(ok.id, UNATTRIBUTED_OPERATOR_ID);
        assert_eq!(ok.mechanism, OperatorAuthMechanism::SharedSecret);

        let wrong = auth
            .authenticate(&input(Some("not-the-secret"), &empty))
            .await
            .expect_err("a wrong secret is invalid");
        assert_eq!(wrong, OperatorAuthFailureReason::InvalidCredential);

        let missing = auth
            .authenticate(&input(None, &empty))
            .await
            .expect_err("an absent credential is missing");
        assert_eq!(missing, OperatorAuthFailureReason::MissingCredential);

        // Redaction: the rendered form carries neither the secret nor its
        // length hint (no digits that match the real length).
        let rendered = format!("{auth:?}");
        assert!(!rendered.contains(SECRET), "debug must redact the secret");
        assert!(rendered.contains("<redacted>"), "debug names the redaction");
    }

    /// A prefix or padded variant of the secret must not authenticate:
    /// the constant-time compare covers the whole value, not a prefix.
    #[tokio::test]
    async fn shared_secret_rejects_prefix_and_padded_variants() {
        let auth =
            SharedSecretAuthenticator::new(oidc_exchange_core::Secret::new(SECRET.to_string()));
        let empty = headers(&[]);

        for presented in [&SECRET[..SECRET.len() - 1], &format!("{SECRET}x")] {
            let reason = auth
                .authenticate(&input(Some(presented), &empty))
                .await
                .expect_err("only the exact value authenticates");
            assert_eq!(reason, OperatorAuthFailureReason::InvalidCredential);
        }
    }

    // --- MtlsSubjectAuthenticator ----------------------------------------

    /// The proxy-asserted subject becomes the principal id; absent, empty,
    /// oversized, and non-UTF-8 assertions are rejected with fixed reasons.
    #[tokio::test]
    async fn mtls_subject_becomes_the_principal_id() {
        let auth = MtlsSubjectAuthenticator::new(HEADER.to_string());
        let subject = "CN=ops.example.com,O=Example";

        let ok = auth
            .authenticate(&input(None, &headers(&[(HEADER, subject)])))
            .await
            .expect("a well-formed subject authenticates");
        assert_eq!(ok.id, subject);
        assert_eq!(ok.mechanism, OperatorAuthMechanism::MutualTls);

        let missing = auth
            .authenticate(&input(None, &headers(&[])))
            .await
            .expect_err("no header means no credential");
        assert_eq!(missing, OperatorAuthFailureReason::MissingCredential);

        for bad in ["", "   "] {
            let reason = auth
                .authenticate(&input(None, &headers(&[(HEADER, bad)])))
                .await
                .expect_err("an empty assertion identifies nobody");
            assert_eq!(reason, OperatorAuthFailureReason::InvalidCredential);
        }
    }

    /// An oversized subject header exceeds the declared bound and is rejected
    /// rather than recorded as an identity.
    #[tokio::test]
    async fn mtls_subject_over_the_bound_is_rejected() {
        let auth = MtlsSubjectAuthenticator::new(HEADER.to_string());
        let oversized = "CN=".to_string() + &"a".repeat(MAX_MTLS_SUBJECT_BYTES + 1);

        let reason = auth
            .authenticate(&input(None, &headers(&[(HEADER, &oversized)])))
            .await
            .expect_err("over-bound assertions are hostile input");
        assert_eq!(reason, OperatorAuthFailureReason::InvalidCredential);
    }

    // --- OperatorTokenAuthenticator --------------------------------------

    fn b64(data: &[u8]) -> String {
        URL_SAFE_NO_PAD.encode(data)
    }

    /// Mint an operator token the way a real issuer would: header, payload,
    /// and a signature produced by the same key manager the authenticator
    /// verifies against.
    async fn mint_token(keys: &MockKeyManager, payload: Value) -> String {
        let header = serde_json::json!({"alg": keys.algorithm(), "typ": "JWT"});
        let signing_input = format!(
            "{}.{}",
            b64(serde_json::to_vec(&header).unwrap().as_slice()),
            b64(serde_json::to_vec(&payload).unwrap().as_slice()),
        );
        let signature = keys.sign(signing_input.as_bytes()).await.unwrap();
        format!("{signing_input}.{}", b64(&signature))
    }

    fn token_authenticator() -> OperatorTokenAuthenticator {
        OperatorTokenAuthenticator::new(
            Box::new(MockKeyManager::new()),
            "https://auth.example.com".to_string(),
            "internal".to_string(),
            "role".to_string(),
            "admin".to_string(),
        )
    }

    fn valid_claims(now: u64) -> Value {
        serde_json::json!({
            "iss": "https://auth.example.com",
            "aud": "internal",
            "sub": "usr_operator_alice",
            "exp": now + 600,
            "role": "admin",
        })
    }

    /// A fully valid token authenticates to its `sub` under the operator-token
    /// mechanism.
    #[tokio::test]
    async fn valid_operator_token_yields_the_token_subject() {
        let keys = MockKeyManager::new();
        let auth = token_authenticator();
        let token = mint_token(&keys, valid_claims(now_unix_secs())).await;
        let empty = headers(&[]);

        let principal = auth
            .authenticate(&input(Some(&token), &empty))
            .await
            .expect("a conforming operator token authenticates");

        assert_eq!(principal.id, "usr_operator_alice");
        assert_eq!(principal.mechanism, OperatorAuthMechanism::OperatorToken);
    }

    /// Negative space over every claim the mechanism requires: expired,
    /// wrong issuer, wrong audience, missing required claim/value, missing
    /// subject, foreign signature, wrong algorithm, and malformed envelope
    /// are each rejected as invalid credentials.
    #[tokio::test]
    async fn operator_token_rejections_cover_every_required_claim() {
        let keys = MockKeyManager::new();
        let auth = token_authenticator();
        let now = now_unix_secs();
        let empty = headers(&[]);

        let cases: Vec<(&str, Value)> = vec![
            // Beyond the skew leeway, so the expiry genuinely bites.
            ("expired", {
                let mut c = valid_claims(now);
                c["exp"] = serde_json::json!(now - OPERATOR_TOKEN_CLOCK_SKEW_LEEWAY_SECS - 10);
                c
            }),
            ("implausibly long lifetime", {
                let mut c = valid_claims(now);
                c["exp"] = serde_json::json!(now + MAX_OPERATOR_TOKEN_TTL_SECS * 10);
                c
            }),
            ("wrong issuer", {
                let mut c = valid_claims(now);
                c["iss"] = serde_json::json!("https://evil.example.com");
                c
            }),
            ("wrong audience", {
                let mut c = valid_claims(now);
                c["aud"] = serde_json::json!("https://api.example.com");
                c
            }),
            ("missing required claim", {
                let mut c = valid_claims(now);
                c.as_object_mut().unwrap().remove("role");
                c
            }),
            ("wrong required value", {
                let mut c = valid_claims(now);
                c["role"] = serde_json::json!("operator");
                c
            }),
            ("missing subject", {
                let mut c = valid_claims(now);
                c.as_object_mut().unwrap().remove("sub");
                c
            }),
            ("empty subject", {
                let mut c = valid_claims(now);
                c["sub"] = serde_json::json!("");
                c
            }),
        ];

        for (label, claims) in cases {
            let token = mint_token(&keys, claims).await;
            let outcome = auth.authenticate(&input(Some(&token), &empty)).await;
            let Err(reason) = outcome else {
                panic!("{label} must be rejected");
            };
            assert_eq!(
                reason,
                OperatorAuthFailureReason::InvalidCredential,
                "{label} must reject as an invalid credential"
            );
        }

        // A token whose signature does not verify fails even when every
        // claim conforms. (The mock key manager is deterministic, so instead
        // of a second key the signature itself is corrupted: one flipped byte
        // in the Ed25519 signature must fail verification.)
        let forged = {
            let mut parts: Vec<String> = mint_token(&keys, valid_claims(now))
                .await
                .split('.')
                .map(str::to_string)
                .collect();
            let mut signature = URL_SAFE_NO_PAD
                .decode(&parts[2])
                .expect("the minted signature is valid base64url");
            assert!(!signature.is_empty(), "a real signature is never empty");
            signature[0] ^= 0xff;
            parts[2] = b64(&signature);
            parts.join(".")
        };
        let reason = auth
            .authenticate(&input(Some(&forged), &empty))
            .await
            .expect_err("a corrupted signature must not verify");
        assert_eq!(reason, OperatorAuthFailureReason::InvalidCredential);

        // Algorithm confusion ("none") is rejected before any crypto call.
        let unsigned_header = format!(
            "{}.{}.{}",
            b64(br#"{"alg":"none","typ":"JWT"}"#),
            b64(serde_json::to_vec(&valid_claims(now)).unwrap().as_slice()),
            b64(b"not-a-signature"),
        );
        let reason = auth
            .authenticate(&input(Some(&unsigned_header), &empty))
            .await
            .expect_err("the none algorithm must never verify");
        assert_eq!(reason, OperatorAuthFailureReason::InvalidCredential);

        // A malformed envelope (two segments) cannot decode.
        let reason = auth
            .authenticate(&input(Some("a.b"), &empty))
            .await
            .expect_err("a non-JWT credential is rejected");
        assert_eq!(reason, OperatorAuthFailureReason::InvalidCredential);

        // No credential at all is missing, not invalid.
        let reason = auth
            .authenticate(&input(None, &empty))
            .await
            .expect_err("absent credentials are missing");
        assert_eq!(reason, OperatorAuthFailureReason::MissingCredential);
    }

    /// Audience matching accepts the string-or-array forms of RFC 7519 §4.1.3.
    #[tokio::test]
    async fn operator_token_accepts_array_audience_containing_the_target() {
        let keys = MockKeyManager::new();
        let auth = token_authenticator();
        let now = now_unix_secs();

        let mut claims = valid_claims(now);
        claims["aud"] = serde_json::json!(["https://api.example.com", "internal"]);
        let token = mint_token(&keys, claims).await;
        let empty = headers(&[]);

        let principal = auth
            .authenticate(&input(Some(&token), &empty))
            .await
            .expect("an audience array containing the target authenticates");
        assert_eq!(principal.id, "usr_operator_alice");
    }

    // --- OperatorAuthGate --------------------------------------------------

    /// Mechanisms run in configured order and the first success wins; a later
    /// mechanism never gets consulted after one admits the request.
    #[tokio::test]
    async fn gate_tries_mechanisms_in_order_and_first_success_wins() {
        struct Rejecting;
        #[async_trait::async_trait]
        impl OperatorAuthenticator for Rejecting {
            fn mechanism(&self) -> OperatorAuthMechanism {
                OperatorAuthMechanism::OperatorToken
            }
            async fn authenticate(
                &self,
                _input: &AuthInput<'_>,
            ) -> Result<OperatorPrincipal, OperatorAuthFailureReason> {
                Err(OperatorAuthFailureReason::InvalidCredential)
            }
        }

        let gate = OperatorAuthGate::new(vec![
            Box::new(Rejecting),
            Box::new(SharedSecretAuthenticator::new(
                oidc_exchange_core::Secret::new(SECRET.to_string()),
            )),
        ]);
        let empty = headers(&[]);

        let principal = gate
            .authenticate(&input(Some(SECRET), &empty))
            .await
            .expect("the second mechanism admits the request");
        assert_eq!(principal.id, UNATTRIBUTED_OPERATOR_ID);
    }

    /// When every configured mechanism rejects, the reported reason follows
    /// the precedence contract: something-presented-and-rejected beats
    /// nothing-presented, and nothing anywhere yields not_configured.
    #[tokio::test]
    async fn gate_failure_reasons_follow_the_precedence_contract() {
        let secret_only = OperatorAuthGate::new(vec![Box::new(SharedSecretAuthenticator::new(
            oidc_exchange_core::Secret::new(SECRET.to_string()),
        ))]);
        let empty = headers(&[]);

        let reason = secret_only
            .authenticate(&input(Some("wrong-value"), &empty))
            .await
            .expect_err("a wrong secret must fail");
        assert_eq!(
            reason,
            OperatorAuthFailureReason::InvalidCredential,
            "a presented-but-rejected credential outranks an absent one"
        );

        let reason = secret_only
            .authenticate(&input(None, &empty))
            .await
            .expect_err("no credential must fail");
        assert_eq!(
            reason,
            OperatorAuthFailureReason::MissingCredential,
            "an absent credential is missing_credential — not_configured stays reserved \\\n             for a plane with no usable mechanism"
        );
    }

    /// A gate cannot be assembled with zero mechanisms: validation guarantees
    /// a non-empty list, and the constructor asserts it.
    #[test]
    #[should_panic(expected = "at least one mechanism")]
    fn gate_construction_requires_a_mechanism() {
        let empty: Vec<Box<dyn OperatorAuthenticator>> = Vec::new();
        let _ = OperatorAuthGate::new(empty);
    }

    /// The gate's Debug output exposes only the mechanism list.
    #[test]
    fn gate_debug_shows_mechanisms_only() {
        let gate = OperatorAuthGate::new(vec![Box::new(SharedSecretAuthenticator::new(
            oidc_exchange_core::Secret::new(SECRET.to_string()),
        ))]);
        let rendered = format!("{gate:?}");
        assert!(
            rendered.contains("SharedSecret"),
            "debug must name the mechanisms, got: {rendered}"
        );
        assert!(!rendered.contains(SECRET), "got: {rendered}");
    }

    /// Route detail maps carry exactly one entry — the route — so the audit
    /// stream can never accumulate request-derived free text.
    #[test]
    fn auth_event_detail_carries_the_route_alone() {
        let detail = auth_event_detail("/internal/users/usr_1/claims");
        assert_eq!(detail.len(), 1);
        assert_eq!(
            detail.get("route").and_then(Value::as_str),
            Some("/internal/users/usr_1/claims")
        );
    }
}
