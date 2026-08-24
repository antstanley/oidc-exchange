//! Assertion binding and nonce minting for the direct ID-token grant.
//!
//! Every accepted ID token passes [`bind`] after its provider validation
//! branch and before user lookup, on both exchange paths. Binding runs five
//! controls once, in the order fixed by `03-service-flows.md` → Token
//! exchange step 3: lifetime ceiling, `azp`, applicable `at_hash`,
//! direct-grant nonce consumption, then the assertion single-use marker. A
//! control rejection is reported as [`AssertionBindError::Rejected`] carrying
//! the failed control's name so the caller can audit `ValidationFailed` and
//! map to `InvalidGrant`; single-use store failures surface as
//! [`AssertionBindError::Store`] and propagate as typed infrastructure
//! errors instead.
//!
//! Nonces minted by [`AppService::mint_nonce`] exist only as 32 random bytes
//! returned once to the caller; storage holds their SHA-256 hex digest keyed
//! under `nonce:`. Assertion markers are stored under
//! `assertion:<provider>:<sha256hex(jti)>`, falling back to a digest of the
//! compact JWT with a `d:` discriminator when the token carries no `jti`.
//! Neither the raw nonce nor any bearer input is ever persisted or logged.

use std::fmt;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256, Sha384, Sha512};

use crate::domain::{AuditEventType, AuditOutcome, AuditSeverity, IdentityClaims};
use crate::error::{Error, Result};
use crate::ports::SessionRepository;
use crate::service::{create_audit_event, AppService};

/// Random bytes in one minted nonce: 256 bits, base64url-encoded for the wire.
pub const NONCE_BYTES: usize = 32;

/// Storage-key namespace prefix for minted nonces; the remainder is the
/// SHA-256 hex of the nonce value.
pub const NONCE_KEY_PREFIX: &str = "nonce:";

/// Storage-key namespace prefix for assertion-replay markers.
pub const ASSERTION_KEY_PREFIX: &str = "assertion:";

/// Discriminator inserted into an assertion marker key when the token carries
/// no `jti` and the marker digests the compact JWT instead. Keeps a literal
/// `jti` value from ever colliding with a JWT digest.
pub const NO_JTI_DISCRIMINATOR: &str = "d:";

/// Length of the base64url-no-pad encoding of [`NONCE_BYTES`] random bytes
/// (⌈256 bits / 6⌉): the exact wire size every minted nonce must have.
pub const NONCE_B64URL_LEN: usize = 43;

/// The only JWS alg family with no OIDC-defined `at_hash` digest. An
/// `at_hash` claim on an EdDSA-signed assertion cannot be verified and is
/// rejected rather than silently skipped, because accepting an unchecked
/// binding claim would read as enforcement in the audit trail.
pub const EDDSA_SIGNING_ALG: &str = "EdDSA";

/// Audit `detail.check` value for the lifetime-ceiling control.
pub const CHECK_LIFETIME_CEILING: &str = "lifetime_ceiling";
/// Audit `detail.check` value for the `azp` control.
pub const CHECK_AZP: &str = "azp";
/// Audit `detail.check` value for the `at_hash` control.
pub const CHECK_AT_HASH: &str = "at_hash";
/// Audit `detail.check` value for the direct-grant nonce control.
pub const CHECK_NONCE: &str = "nonce";
/// Audit `detail.check` value for the assertion single-use marker control.
pub const CHECK_SINGLE_USE: &str = "single_use";

/// A freshly minted single-use nonce for the direct ID-token grant.
///
/// The value is returned exactly once here; only its SHA-256 hex digest is
/// stored. `expires_in` is the configured `grants.nonce_ttl` in seconds, so
/// clients can bound their own caching.
pub struct MintedNonce {
    /// 32 random bytes, base64url-no-pad. A bearer pre-credential: never log
    /// or persist it in raw form.
    pub nonce: String,
    /// Seconds until the stored digest record expires (`grants.nonce_ttl`).
    pub expires_in: u64,
}

impl fmt::Debug for MintedNonce {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MintedNonce")
            .field("nonce", &"<redacted>")
            .field("expires_in", &self.expires_in)
            .finish()
    }
}

/// Everything [`bind`] needs beyond the verified claims themselves.
///
/// All fields arrive from trusted sources: the provider identity and pinned
/// client come through the `IdentityProvider` port, the access token from the
/// request (direct grant) or the provider's token response (code path), and
/// the ceiling seconds were parsed from validated config by the caller.
pub struct AssertionContext<'a> {
    /// Configured identifier of the provider that verified this assertion
    /// (`provider.provider_id()`); namespaces the replay-marker key.
    pub provider_id: &'a str,
    /// The audience this provider pins tokens to (`provider.client_id()`);
    /// compared against a present `azp` claim.
    pub client_id: &'a str,
    /// Provider access token accompanying the assertion, when one was
    /// presented (`provider_access_token` on the direct grant,
    /// `ProviderTokens.access_token` on the code path). Presence turns the
    /// `at_hash` check from skippable into enforced.
    pub access_token: Option<&'a str>,
    /// The compact-serialized ID token as presented. Used only for the
    /// no-`jti` replay-marker fallback digest.
    pub compact_jwt: &'a str,
    /// Whether this exchange is the direct ID-token grant (the only path that
    /// must burn a server-minted nonce).
    pub require_nonce: bool,
    /// Ceiling on remaining assertion lifetime in seconds, parsed from
    /// `grants.max_assertion_lifetime` by the caller so config errors stay
    /// config errors instead of becoming binding rejections.
    pub max_assertion_secs: u64,
}

/// Why a binding control rejected an assertion.
///
/// `check` names the failed control for the audit event's `detail.check`;
/// `reason` is generic and safe for both the audit trail and the client-facing
/// `invalid_grant` description — it names the control, never token contents,
/// nonces, or subjects.
#[derive(Debug, Clone)]
pub struct AssertionRejection {
    /// The failed control's stable name (e.g. [`CHECK_NONCE`]).
    pub check: &'static str,
    /// Generic failure reason naming the control only.
    pub reason: String,
}

/// Terminal outcome of [`bind`]: either a control rejected the assertion (the
/// caller audits `ValidationFailed`/`Warning` and answers `InvalidGrant`) or
/// the single-use store itself failed (propagated as-is, mapped to 5xx).
#[derive(Debug)]
pub enum AssertionBindError {
    /// A binding control rejected the assertion before any user lookup.
    Rejected(AssertionRejection),
    /// A `SessionRepository` single-use operation failed; infrastructure.
    Store(Error),
}

/// Result of one binding step: a rejection carries the failed control's name,
/// a store failure carries the typed infrastructure error.
pub type BindingResult<T> = std::result::Result<T, AssertionBindError>;

impl From<AssertionRejection> for AssertionBindError {
    fn from(rejection: AssertionRejection) -> Self {
        AssertionBindError::Rejected(rejection)
    }
}

/// Run every shared binding control over a verified assertion, in the order
/// fixed by the service-flow spec, burning state only as each control passes.
///
/// Order matters: the nonce is consumed before the marker is claimed, so an
/// attacker holding a victim's assertion but no valid nonce can neither
/// complete an exchange nor pin the marker to deny the honest client its own
/// first use. Store failures abort with [`AssertionBindError::Store`] without
/// having necessarily burned the nonce — a partial run never admits a replay,
/// because the marker claim is the last write.
pub async fn bind(
    session_repo: &dyn SessionRepository,
    claims: &IdentityClaims,
    ctx: &AssertionContext<'_>,
) -> BindingResult<()> {
    let now = Utc::now();
    let expires_at = check_lifetime(claims, now, ctx.max_assertion_secs)?;
    check_azp(claims, ctx)?;
    check_at_hash(claims, ctx)?;
    if ctx.require_nonce {
        consume_nonce(session_repo, claims).await?;
    }
    claim_assertion_marker(session_repo, claims, ctx, expires_at).await?;
    Ok(())
}

/// Read a string-valued claim out of the verified raw claims.
fn str_claim<'a>(claims: &'a IdentityClaims, name: &str) -> Option<&'a str> {
    claims.raw_claims.get(name).and_then(Value::as_str)
}

/// Parse the assertion's `exp` claim and enforce the remaining-lifetime
/// controls. Returns the expiry instant, which doubles as the replay marker's
/// `expires_at`: assertions whose `exp` has passed are refused even though
/// validator leeway may still admit them, so the marker never starts dead.
///
/// A missing or unparseable `exp` fails this control: real validators require
/// the claim (jsonwebtoken pins `exp` as required), and the marker could not
/// be bounded without it, so an assertion without a usable `exp` is refused
/// rather than stored open-endedly.
fn check_lifetime(
    claims: &IdentityClaims,
    now: DateTime<Utc>,
    max_assertion_secs: u64,
) -> BindingResult<DateTime<Utc>> {
    let fail = || -> AssertionBindError {
        AssertionRejection {
            check: CHECK_LIFETIME_CEILING,
            reason: "assertion has no usable exp claim".to_string(),
        }
        .into()
    };

    // NumericDate is a number per OIDC Core §2; tolerate float encodings some
    // libraries emit, refuse everything else.
    let exp_secs: i64 = match claims.raw_claims.get("exp") {
        Some(Value::Number(n)) => n
            .as_i64()
            .or_else(|| n.as_f64().map(|f| f as i64))
            .ok_or_else(fail)?,
        _ => return Err(fail()),
    };
    assert!(now.timestamp() > 0, "clock must be past the epoch");

    let remaining = exp_secs - now.timestamp();
    // Strictly positive: validators admit tokens whose `exp` passed within
    // their leeway (jsonwebtoken defaults to 60s), but a marker written with
    // `expires_at <= now` is born dead — every store treats it as absent, so
    // replay protection would silently vanish.
    if remaining <= 0 {
        return Err(AssertionRejection {
            check: CHECK_LIFETIME_CEILING,
            reason: "assertion is already expired".to_string(),
        }
        .into());
    }
    if remaining > max_assertion_secs as i64 {
        return Err(AssertionRejection {
            check: CHECK_LIFETIME_CEILING,
            reason: "assertion lifetime exceeds the configured maximum".to_string(),
        }
        .into());
    }

    let expires_at = DateTime::<Utc>::from_timestamp(exp_secs, 0).ok_or_else(fail)?;
    Ok(expires_at)
}

/// Enforce the `azp` rule: whenever `azp` is present it must equal the
/// provider's configured client id, and a multi-valued `aud` array requires
/// `azp`. A token minted for a sibling client of the same provider is caught
/// here even though its signature verifies.
fn check_azp(claims: &IdentityClaims, ctx: &AssertionContext<'_>) -> BindingResult<()> {
    match str_claim(claims, "azp") {
        Some(azp) => {
            if azp != ctx.client_id {
                return Err(AssertionRejection {
                    check: CHECK_AZP,
                    reason: "azp does not match this client".to_string(),
                }
                .into());
            }
        }
        None => {
            let aud_is_multi =
                matches!(claims.raw_claims.get("aud"), Some(Value::Array(a)) if a.len() > 1);
            if aud_is_multi {
                return Err(AssertionRejection {
                    check: CHECK_AZP,
                    reason: "multi-audience assertion requires azp".to_string(),
                }
                .into());
            }
        }
    }
    Ok(())
}

/// Enforce the `at_hash` rule.
///
/// - Any `at_hash` on an EdDSA-signed assertion is rejected outright: OIDC
///   Core defines no digest for EdDSA, so the claim cannot be checked.
/// - With an access token present and a verifiable signing-alg family, the
///   claim must equal the base64url(no-pad) of the left-most half of the
///   access token's digest (OIDC Core §3.1.3.6).
/// - An `at_hash` with no accompanying access token is not verifiable and is
///   skipped; an assertion with no `at_hash` claim at all has nothing to
///   verify and also skips this control.
/// - A `signing_alg` outside the known digest families (and outside EdDSA)
///   fails closed when verification would otherwise apply.
fn check_at_hash(claims: &IdentityClaims, ctx: &AssertionContext<'_>) -> BindingResult<()> {
    let Some(claimed_at_hash) = str_claim(claims, "at_hash") else {
        return Ok(());
    };

    if claims.signing_alg == EDDSA_SIGNING_ALG {
        return Err(AssertionRejection {
            check: CHECK_AT_HASH,
            reason: "at_hash is unverifiable on an EdDSA-signed assertion".to_string(),
        }
        .into());
    }

    let Some(access_token) = ctx.access_token else {
        // Skipped by spec: nothing was presented to verify the claim against.
        return Ok(());
    };

    let Some(expected) = at_hash_value(&claims.signing_alg, access_token) else {
        return Err(AssertionRejection {
            check: CHECK_AT_HASH,
            reason: "no verifiable at_hash digest for this signing algorithm".to_string(),
        }
        .into());
    };

    if !constant_time_eq(expected.as_bytes(), claimed_at_hash.as_bytes()) {
        return Err(AssertionRejection {
            check: CHECK_AT_HASH,
            reason: "at_hash does not match the accompanying access token".to_string(),
        }
        .into());
    }
    Ok(())
}

/// Compute the expected `at_hash` for an access token under the given JWS
/// algorithm name: SHA-256/384/512 selected by the algorithm-name suffix, the
/// left-most half of the digest, base64url-no-pad. `None` when the algorithm
/// names no supported digest family.
fn at_hash_value(signing_alg: &str, access_token: &str) -> Option<String> {
    let half: Vec<u8> = if signing_alg.ends_with("256") {
        Sha256::digest(access_token.as_bytes())[..16].to_vec()
    } else if signing_alg.ends_with("384") {
        Sha384::digest(access_token.as_bytes())[..24].to_vec()
    } else if signing_alg.ends_with("512") {
        Sha512::digest(access_token.as_bytes())[..32].to_vec()
    } else {
        return None;
    };
    Some(URL_SAFE_NO_PAD.encode(half))
}

/// Byte-equality without data-dependent early exit. The compared values are
/// digests derived from a bearer credential, so a timing oracle on the first
/// differing byte would leak information about the credential-derived value.
/// Length differences still exit early — digest lengths are public constants,
/// and an empty or short attacker-supplied claim simply mismatches here.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    debug_assert!(!a.is_empty(), "the computed digest is always non-empty");
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Verify and burn the direct grant's nonce in one atomic store operation:
/// absent, expired, and already-burned nonces are indistinguishable and all
/// reject. Only the SHA-256 hex of the presented value ever reaches storage.
async fn consume_nonce(
    session_repo: &dyn SessionRepository,
    claims: &IdentityClaims,
) -> BindingResult<()> {
    let Some(nonce) = str_claim(claims, "nonce").filter(|n| !n.is_empty()) else {
        return Err(AssertionRejection {
            check: CHECK_NONCE,
            reason: "assertion carries no usable nonce".to_string(),
        }
        .into());
    };

    let nonce_digest = hex::encode(Sha256::digest(nonce.as_bytes()));
    let key = format!("{NONCE_KEY_PREFIX}{nonce_digest}");
    let burned = session_repo
        .take_single_use(&key)
        .await
        .map_err(AssertionBindError::Store)?;
    if !burned {
        return Err(AssertionRejection {
            check: CHECK_NONCE,
            reason: "nonce is missing, expired, or already used".to_string(),
        }
        .into());
    }
    Ok(())
}

/// Claim the assertion-replay marker: `put_single_use` returning `false` means
/// the exact assertion was spent before, which is a replay. The key digests
/// the `jti` when present, else the compact JWT behind the `d:` discriminator;
/// the record expires at the assertion's own `exp`.
async fn claim_assertion_marker(
    session_repo: &dyn SessionRepository,
    claims: &IdentityClaims,
    ctx: &AssertionContext<'_>,
    expires_at: DateTime<Utc>,
) -> BindingResult<()> {
    let key = assertion_marker_key(ctx, claims);
    let claimed = session_repo
        .put_single_use(&key, expires_at)
        .await
        .map_err(AssertionBindError::Store)?;
    if !claimed {
        return Err(AssertionRejection {
            check: CHECK_SINGLE_USE,
            reason: "assertion has already been used".to_string(),
        }
        .into());
    }
    Ok(())
}

/// Build the provider-namespaced replay-marker key. Both branches store only
/// SHA-256 hex digests — never the raw `jti` value or the raw JWT.
fn assertion_marker_key(ctx: &AssertionContext<'_>, claims: &IdentityClaims) -> String {
    match str_claim(claims, "jti").filter(|jti| !jti.is_empty()) {
        Some(jti) => format!(
            "{ASSERTION_KEY_PREFIX}{}:{}",
            ctx.provider_id,
            hex::encode(Sha256::digest(jti.as_bytes()))
        ),
        None => format!(
            "{ASSERTION_KEY_PREFIX}{}:{NO_JTI_DISCRIMINATOR}{}",
            ctx.provider_id,
            hex::encode(Sha256::digest(ctx.compact_jwt.as_bytes()))
        ),
    }
}

impl AppService {
    /// Mint a single-use nonce for the direct ID-token grant: 32 random bytes
    /// returned base64url-no-pad, stored only as the SHA-256 hex digest under
    /// [`NONCE_KEY_PREFIX`], expiring after `grants.nonce_ttl`.
    ///
    /// A `put_single_use` miss is a 256-bit collision — surfaced as
    /// `StoreError` rather than retried, because silently regenerating would
    /// turn an invariant violation into unbounded writes.
    pub async fn mint_nonce(&self) -> Result<MintedNonce> {
        let ttl_secs = parse_nonce_ttl_secs(self)?;

        let nonce_bytes: [u8; NONCE_BYTES] = rand::random();
        let nonce = URL_SAFE_NO_PAD.encode(nonce_bytes);
        // Postcondition: the wire encoding has the exact documented size, so
        // a client-side length sanity check can never reject a minted nonce.
        assert_eq!(
            nonce.len(),
            NONCE_B64URL_LEN,
            "base64url of 32 bytes must be 43 chars"
        );

        let nonce_digest = hex::encode(Sha256::digest(nonce.as_bytes()));
        assert_eq!(
            nonce_digest.len(),
            SHA256_HEX_LEN,
            "SHA-256 hex digests are 64 chars"
        );
        let key = format!("{NONCE_KEY_PREFIX}{nonce_digest}");
        let expires_at = Utc::now() + chrono::Duration::seconds(ttl_secs as i64);

        let claimed = self.session_repo.put_single_use(&key, expires_at).await?;
        if !claimed {
            return Err(Error::StoreError {
                detail: "single-use nonce key collision".to_string(),
            });
        }

        Ok(MintedNonce {
            nonce,
            expires_in: ttl_secs,
        })
    }

    /// Emit the canonical `ValidationFailed`/`Warning` audit event for a
    /// binding rejection, tagging the failed control in `detail.check`, then
    /// translate the rejection into the domain error callers map to
    /// `invalid_grant`. Store failures never reach this method.
    pub(crate) async fn audit_binding_rejection(
        &self,
        rejection: &AssertionRejection,
        provider: Option<&str>,
        ip_address: Option<&str>,
        user_agent: Option<&str>,
    ) -> Result<()> {
        let mut event = create_audit_event(
            AuditEventType::ValidationFailed,
            AuditSeverity::Warning,
            AuditOutcome::Failure {
                reason: rejection.reason.clone(),
            },
            None,
            provider.map(str::to_string),
            ip_address.map(str::to_string),
            user_agent.map(str::to_string),
        );
        event.detail.insert(
            "check".to_string(),
            serde_json::Value::String(rejection.check.to_string()),
        );
        self.emit_audit(event).await
    }
}

/// Length of a SHA-256 hex digest string.
const SHA256_HEX_LEN: usize = 64;

/// Parse `grants.nonce_ttl` into seconds. Kept separate so `mint_nonce` stays
/// inside the function-size review gate and the config dependency is explicit.
fn parse_nonce_ttl_secs(service: &AppService) -> Result<u64> {
    let secs = service.config.grants.nonce_ttl.as_secs();
    assert!(
        secs <= u64::MAX / 2,
        "nonce TTL must leave headroom for arithmetic"
    );
    Ok(secs)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn claims_with(extra: &[(&str, Value)]) -> IdentityClaims {
        let mut raw: HashMap<String, Value> = HashMap::new();
        for (name, value) in extra {
            raw.insert((*name).to_string(), value.clone());
        }
        IdentityClaims {
            subject: "subject-under-test".to_string(),
            email: None,
            email_verified: None,
            name: None,
            is_private_email: None,
            signing_alg: "RS256".to_string(),
            raw_claims: raw,
        }
    }

    /// at_hash follows OIDC Core §3.1.3.6 exactly: base64url(no-pad) of the
    /// left-most half of SHA-256(access token ASCII octets).
    #[test]
    fn at_hash_value_matches_oidc_core_rs256_recipe() {
        let access_token = "ya29.a0AfH6SMBxample";
        let expected = URL_SAFE_NO_PAD.encode(&Sha256::digest(access_token.as_bytes())[..16]);
        assert_eq!(
            at_hash_value("RS256", access_token).as_deref(),
            Some(expected.as_str())
        );
        // Negative space: a different token must not produce the same digest.
        assert_ne!(
            at_hash_value("RS256", "other-token"),
            at_hash_value("RS256", access_token)
        );
    }

    #[test]
    fn at_hash_value_selects_digest_by_suffix_and_rejects_unknown() {
        let token = "token";
        assert_eq!(
            at_hash_value("ES512", token),
            at_hash_value("PS512", token),
            "same suffix selects the same digest family"
        );
        assert_eq!(
            at_hash_value("EdDSA", token),
            None,
            "EdDSA names no digest family"
        );
        // base64url of a half-digest is always a whole number of 6-bit groups
        // for the supported families (16/24/32 bytes), so output never pads.
        let sha384 = at_hash_value("RS384", token).expect("RS384 is verifiable");
        assert_eq!(
            sha384.len() % 4,
            0,
            "half-of-48-bytes encodes without padding"
        );
        assert_eq!(at_hash_value("", token), None, "empty alg names no family");
    }

    #[test]
    fn constant_time_eq_differs_on_length_and_content() {
        assert!(constant_time_eq(b"abcdef", b"abcdef"));
        assert!(
            !constant_time_eq(b"abcdef", b"abcdeg"),
            "one-bit content diff must mismatch"
        );
        assert!(
            !constant_time_eq(b"abcdef", b"abcde"),
            "length diff must mismatch"
        );
        // An attacker-supplied empty claim mismatches the computed digest.
        // (The reverse order is unreachable: the computed side is never empty,
        // which its debug_assert enforces.)
        assert!(!constant_time_eq(b"digest", b""));
    }

    #[test]
    fn assertion_marker_key_digests_jti_when_present() {
        let ctx = AssertionContext {
            provider_id: "google",
            client_id: "client",
            access_token: None,
            compact_jwt: "header.payload.signature",
            require_nonce: false,
            max_assertion_secs: 3600,
        };
        let claims = claims_with(&[("jti", Value::String("jti-value".into()))]);

        let key = assertion_marker_key(&ctx, &claims);

        let expected_digest = hex::encode(Sha256::digest(b"jti-value"));
        assert_eq!(key, format!("assertion:google:{expected_digest}"));
        assert!(
            !key.contains("jti-value"),
            "raw jti never appears in the key"
        );
    }

    #[test]
    fn assertion_marker_key_falls_back_to_discriminated_jwt_digest_without_jti() {
        let ctx = AssertionContext {
            provider_id: "apple",
            client_id: "client",
            access_token: None,
            compact_jwt: "h.p.s",
            require_nonce: false,
            max_assertion_secs: 3600,
        };
        let claims = claims_with(&[]);

        let key = assertion_marker_key(&ctx, &claims);

        let jwt_digest = hex::encode(Sha256::digest(b"h.p.s"));
        assert_eq!(key, format!("assertion:apple:d:{jwt_digest}"));

        // A literal `jti` whose value happens to equal the JWT digest cannot
        // collide with the fallback branch: the discriminator separates them.
        let impostor = claims_with(&[("jti", Value::String(jwt_digest.clone()))]);
        assert_ne!(
            assertion_marker_key(&ctx, &impostor),
            key,
            "d: discriminator keeps literal-jti keys distinct from fallback keys"
        );
    }

    /// An empty-string `jti` is treated as absent so two assertions that both
    /// claim nothing cannot share one marker via an empty digest.
    #[test]
    fn assertion_marker_key_treats_empty_jti_as_absent() {
        let ctx = AssertionContext {
            provider_id: "p",
            client_id: "client",
            access_token: None,
            compact_jwt: "the-token",
            require_nonce: false,
            max_assertion_secs: 60,
        };
        let empty_jti = claims_with(&[("jti", Value::String(String::new()))]);

        assert_eq!(
            assertion_marker_key(&ctx, &empty_jti),
            assertion_marker_key(&ctx, &claims_with(&[])),
            "empty jti must fall through to the compact-JWT digest"
        );
    }
}
