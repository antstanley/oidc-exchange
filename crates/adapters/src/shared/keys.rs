//! The one place a JWK becomes a verification key.
//!
//! Two independent `find_jwk` copies — one per provider — selected on `kid`
//! alone, never consulted `use`, `key_ops`, or `kty`, and had already drifted on
//! algorithm handling (a nine-arm `alg` match with an inference fallback versus
//! a two-arm match with none). Concentrating eligibility in this constructor
//! makes the filter worth testing exhaustively, which two copies can never be.
//!
//! Policy lives with the caller, mechanics live here: the constructor takes the
//! provider's admitted-algorithm set as a parameter and applies exactly one
//! rulebook. The generic adapter admits the nine JWS algorithms it always has;
//! Apple admits `{RS256, ES256}`; neither set is derived from the other.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use jsonwebtoken::{Algorithm, DecodingKey};
use oidc_exchange_core::error::{Error, Result};

/// The verification value a resolved `kid` hands to signature checking:
/// the decoding key together with the algorithm it must be verified under,
/// carried as data so no caller re-derives either from an untrusted header.
#[derive(Clone, Debug)]
pub struct VerificationKey {
    kid: String,
    algorithm: Algorithm,
    decoding_key: Arc<DecodingKey>,
}

impl VerificationKey {
    /// The `kid` this key was published under.
    pub fn kid(&self) -> &str {
        &self.kid
    }

    /// The algorithm signatures from this key verify under.
    pub fn algorithm(&self) -> Algorithm {
        self.algorithm
    }

    /// The decoding key to verify with.
    pub fn decoding_key(&self) -> &DecodingKey {
        &self.decoding_key
    }
}

/// An immutable set of verification keys built from one fetched JWKS document.
///
/// Eligibility was decided entirely at construction: everything left in the set
/// passed the purpose filter (`use`/`key_ops`), the algorithm filter (declared
/// algorithms parse, belong to the admitted set, and agree with their key type),
/// and decoded into real key material. A `kid` lookup therefore either yields a
/// usable [`VerificationKey`] or nothing — there is no third state where a key
/// resolves but turns out unusable mid-validation.
#[derive(Debug)]
pub struct VerificationKeySet {
    by_kid: HashMap<String, Arc<VerificationKey>>,
}

impl VerificationKeySet {
    /// Build a key set from a fetched JWKS document and the provider's admitted
    /// algorithm set.
    ///
    /// Ineligible entries are dropped, not fatal: a JWKS that mixes signing keys
    /// with encryption or encryption-algorithm keys stays usable for its valid
    /// entries, and a lookup for a dropped `kid` misses (and fails closed)
    /// rather than poisoning every validation. Structural problems with the
    /// *document* — no `keys` array, or several eligible entries claiming one
    /// `kid`, where selection would be ambiguous — are errors: the provider's
    /// response cannot be trusted as a whole.
    pub fn from_jwks(
        provider: &str,
        jwks: &serde_json::Value,
        admitted_algorithms: &'static [Algorithm],
    ) -> Result<Self> {
        assert!(
            !admitted_algorithms.is_empty(),
            "an admitted-algorithm policy must admit at least one algorithm"
        );

        let entries = jwks
            .get("keys")
            .and_then(|keys| keys.as_array())
            .ok_or_else(|| Error::ProviderError {
                provider: provider.to_string(),
                detail: "JWKS response missing 'keys' array".into(),
            })?;

        let mut by_kid: HashMap<String, Arc<VerificationKey>> = HashMap::new();
        for entry in entries {
            // An entry that is not an object cannot be a JWK; drop it like any
            // other ineligible entry rather than failing the whole document.
            let Some(jwk) = entry.as_object() else {
                continue;
            };

            let Some(key) = build_verification_key(provider, jwk, admitted_algorithms) else {
                continue;
            };

            if let Some(existing) = by_kid.get(key.kid()) {
                debug_assert_eq!(existing.kid(), key.kid());
                return Err(Error::ProviderError {
                    provider: provider.to_string(),
                    detail: format!("JWKS carries several eligible keys for kid {:?}", key.kid()),
                });
            }
            by_kid.insert(key.kid().to_string(), Arc::new(key));
        }

        Ok(Self { by_kid })
    }

    /// Look up a `kid`, returning the single eligible key published under it.
    ///
    /// Order-independent by construction: array position decided nothing, so a
    /// duplicate-`kid` JWKS whose ineligible entry comes first behaves exactly
    /// like its mirror image.
    pub fn get(&self, kid: &str) -> Option<Arc<VerificationKey>> {
        assert!(!kid.is_empty(), "kid must not be empty");
        self.by_kid.get(kid).cloned()
    }

    /// How many eligible keys the set holds.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.by_kid.len()
    }
}

/// Apply the full eligibility rulebook to one JWK entry, yielding a
/// [`VerificationKey`] — or `None` when any rule rejects the entry.
///
/// Rejections are silent by design here; the caller-level effect of dropping a
/// key is that its `kid` simply does not resolve. The rules, in order:
///
/// 1. `use`, when present, must be `"sig"` (RFC 7517 §4.2).
/// 2. `key_ops`, when present, must contain `"verify"` (RFC 7517 §4.3).
/// 3. `kty` must be a supported asymmetric type; symmetric (`oct`) keys are
///    never candidates.
/// 4. A declared `alg` must parse as a known JWS algorithm, belong to the
///    caller's admitted set, and agree with `kty`/`crv`. Unknown declared
///    algorithms are rejected outright — they are never treated as absent.
/// 5. A genuinely absent `alg` is inferred from trusted key material only:
///    RSA → RS256, EC P-256 → ES256, EC P-384 → ES384, OKP Ed25519 → EdDSA.
/// 6. The inferred-or-admitted algorithm must survive the same admission check
///    as a declared one, so inference can never widen a provider's policy.
fn build_verification_key(
    provider: &str,
    jwk: &serde_json::Map<String, serde_json::Value>,
    admitted_algorithms: &'static [Algorithm],
) -> Option<VerificationKey> {
    let _ = provider; // used for error context below once material decoding fails

    // RFC 7517 §4.2: purpose is binding when declared, permissive when absent.
    // Many identity providers omit `use`; rejecting those keys would break
    // working deployments, while treating a declared purpose as decoration is
    // what let an encryption key verify an identity assertion.
    if let Some(use_value) = jwk.get("use").and_then(|v| v.as_str()) {
        if use_value != "sig" {
            return None;
        }
    }

    // RFC 7517 §4.3: operations bind when declared, likewise permissive when
    // absent. Absent `key_ops` must not imply "no operations allowed".
    if let Some(ops) = jwk.get("key_ops").and_then(|v| v.as_array()) {
        let verifies = ops.iter().any(|op| op.as_str() == Some("verify"));
        if !verifies {
            return None;
        }
    }

    let kty = jwk.get("kty").and_then(|v| v.as_str())?;
    // Symmetric material verifies anything presented under the same secret —
    // the classic algorithm-confusion target — so it has no arm at all.
    if kty == "oct" {
        return None;
    }

    let crv = jwk.get("crv").and_then(|v| v.as_str());

    // Decide the candidate algorithm: from the declared member when present,
    // otherwise from the narrow inference table. Declared-unknown and declared-
    // outside-policy take the same exit (drop); only genuine absence infers.
    let candidate = match jwk.get("alg").and_then(|v| v.as_str()) {
        Some(declared) => {
            let parsed = parse_algorithm(declared)?;
            check_family(parsed, kty, crv)?;
            parsed
        }
        None => infer_algorithm(kty, crv)?,
    };

    // Admission is applied after inference too: an inferred algorithm is a
    // candidate, not an exemption, so inference cannot widen the policy.
    if !admitted_algorithms.contains(&candidate) {
        return None;
    }

    let kid = jwk
        .get("kid")
        .and_then(|v| v.as_str())
        .filter(|kid| !kid.is_empty())?
        .to_string();

    let jwk_value: jsonwebtoken::jwk::Jwk =
        serde_json::from_value(serde_json::Value::Object(jwk.clone())).ok()?;
    let decoding_key = DecodingKey::from_jwk(&jwk_value).ok()?;

    Some(VerificationKey {
        kid,
        algorithm: candidate,
        decoding_key: Arc::new(decoding_key),
    })
}

/// Parse a declared `alg` string into a JWS algorithm.
///
/// Anything unparseable — `RSA-OAEP`, `"none"`, garbage — returns `None`: an
/// unknown declared algorithm is a rejection, never a reason to fall through to
/// inference. HMAC algorithms parse but fail the family check against every
/// asymmetric key type, and `oct` keys were already refused above.
fn parse_algorithm(declared: &str) -> Option<Algorithm> {
    assert!(
        !declared.is_empty(),
        "declared alg strings are non-empty here"
    );
    Algorithm::from_str(declared).ok()
}

/// Require agreement between a declared algorithm and the key's material.
fn check_family(algorithm: Algorithm, kty: &str, crv: Option<&str>) -> Option<()> {
    let agrees = match (algorithm, kty) {
        (Algorithm::RS256 | Algorithm::RS384 | Algorithm::RS512, "RSA") => true,
        (Algorithm::PS256 | Algorithm::PS384 | Algorithm::PS512, "RSA") => true,
        (Algorithm::ES256, "EC") => crv == Some("P-256"),
        (Algorithm::ES384, "EC") => crv == Some("P-384"),
        (Algorithm::EdDSA, "OKP") => crv == Some("Ed25519"),
        _ => false,
    };
    if agrees {
        Some(())
    } else {
        None
    }
}

/// Infer the algorithm of an alg-less JWK from its own trusted material.
///
/// Azure-AD-style JWKS omit `alg`; the algorithm is then derived from the key
/// itself rather than trusted from the token header. The OKP arm requires
/// `crv: Ed25519` explicitly — a bare `kty: OKP` wildcard would land curves
/// that are not signature curves in an algorithm they cannot carry.
fn infer_algorithm(kty: &str, crv: Option<&str>) -> Option<Algorithm> {
    match (kty, crv) {
        ("RSA", _) => Some(Algorithm::RS256),
        ("EC", Some("P-256")) => Some(Algorithm::ES256),
        ("EC", Some("P-384")) => Some(Algorithm::ES384),
        ("OKP", Some("Ed25519")) => Some(Algorithm::EdDSA),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The generic adapter's admitted set lives with the adapter; tests here use
    /// an equivalent local copy so the unit tests stay focused on mechanics.
    const NINE: &[Algorithm] = &[
        Algorithm::RS256,
        Algorithm::RS384,
        Algorithm::RS512,
        Algorithm::ES256,
        Algorithm::ES384,
        Algorithm::PS256,
        Algorithm::PS384,
        Algorithm::PS512,
        Algorithm::EdDSA,
    ];

    const APPLE_SHAPED: &[Algorithm] = &[Algorithm::RS256, Algorithm::ES256];

    fn rsa_jwk() -> serde_json::Value {
        json!({
            "kty": "RSA",
            "kid": "test-rsa",
            "alg": "RS256",
            "use": "sig",
            "n": "0vx7agoebGcQSuuPiLJXZptN9nndrQmbXEps2aiAFbWhM78LhWx4cbbfAAtVT86zwu1RK7aPFFxuhDR1L6tSoc_BJECPebWKRXjBZCiFV4n3oknjhMstn64tZ_2W-5JsGY4Hc5n9yBXArwl93lqt7_RN5w6Cf0h4QyQ5v-65YGjQR0_FDW2QvzqY368QQMicAtaSqzs8KJZgnYb9c7d0zgdAZHzu6qMQvRL5hajrn1n91CbOpbISD08qNLyrdkt-bFTWhAI4vMQFh6WeZu0fM4lFd2NcRwr3XPksINHaQ-G_xBniIqbw0Ls1jF44-csFCur-kEgU8awapJzKnqDKgw",
            "e": "AQAB"
        })
    }

    #[test]
    fn clean_sig_rsa_key_resolves_with_its_algorithm_as_data() {
        let set = VerificationKeySet::from_jwks("p", &json!({"keys": [rsa_jwk()]}), NINE)
            .expect("clean key set builds");

        let vk = set
            .get("test-rsa")
            .expect("the only entry must resolve by kid");
        assert_eq!(vk.algorithm(), Algorithm::RS256);
        assert_eq!(vk.kid(), "test-rsa");
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn use_enc_and_key_ops_without_verify_are_dropped() {
        let enc = json!({
            "kty": "RSA", "kid": "enc-key", "alg": "RS256", "use": "enc",
            "n": rsa_jwk()["n"], "e": "AQAB"
        });
        let ops = json!({
            "kty": "RSA", "kid": "ops-key", "alg": "RS256",
            "key_ops": ["encrypt", "wrapKey"],
            "n": rsa_jwk()["n"], "e": "AQAB"
        });
        let set = VerificationKeySet::from_jwks("p", &json!({"keys": [enc, ops]}), NINE)
            .expect("dropped entries do not break the set");

        assert_eq!(set.len(), 0, "neither entry may remain eligible");
        assert!(set.get("enc-key").is_none());
        assert!(set.get("ops-key").is_none());
    }

    #[test]
    fn key_ops_present_must_contain_verify_but_absent_is_permissive() {
        let without_member = json!({
            "kty": "RSA", "kid": "plain", "alg": "RS256",
            "n": rsa_jwk()["n"], "e": "AQAB"
        });
        let set = VerificationKeySet::from_jwks("p", &json!({"keys": [without_member]}), NINE)
            .expect("absent use/key_ops members are permitted");
        assert_eq!(set.len(), 1);

        let with_verify = json!({
            "kty": "RSA", "kid": "verifyer", "alg": "RS256",
            "key_ops": ["verify"],
            "n": rsa_jwk()["n"], "e": "AQAB"
        });
        let set = VerificationKeySet::from_jwks("p", &json!({"keys": [with_verify]}), NINE)
            .expect("key_ops containing verify is admitted");
        assert!(set.get("verifyer").is_some());
    }

    #[test]
    fn declared_algorithm_outside_or_inconsistent_with_policy_is_dropped() {
        // RSA-OAEP parses in JOSE terms but not as a verification algorithm
        // here: unknown-declared takes the same drop path as out-of-set.
        let oaep = json!({
            "kty": "RSA", "kid": "oaep", "alg": "RSA-OAEP",
            "n": rsa_jwk()["n"], "e": "AQAB"
        });
        // ES256 declared on RSA material: right family name, wrong key type.
        let inconsistent = json!({
            "kty": "RSA", "kid": "inconsistent", "alg": "ES256",
            "n": rsa_jwk()["n"], "e": "AQAB"
        });
        let set = VerificationKeySet::from_jwks("p", &json!({"keys": [oaep, inconsistent]}), NINE)
            .expect("entry drops keep the set intact");
        assert_eq!(set.len(), 0);
        assert!(set.get("oaep").is_none());
        assert!(set.get("inconsistent").is_none());
    }

    #[test]
    fn oct_and_alg_none_never_resolve_on_any_policy() {
        let oct = json!({
            "kty": "oct", "kid": "symmetric", "alg": "HS256",
            "use": "sig", "k": "c2VjcmV0"
        });
        let none = {
            let mut jwk = rsa_jwk();
            jwk["kid"] = json!("none-alg");
            jwk["alg"] = json!("none");
            jwk
        };
        for policy in [NINE, APPLE_SHAPED] {
            let set = VerificationKeySet::from_jwks(
                "p",
                &json!({"keys": [oct.clone(), none.clone()]}),
                policy,
            )
            .expect("entries drop without failing the document");
            assert_eq!(set.len(), 0, "{policy:?} must admit neither entry");
            assert!(set.get("symmetric").is_none());
            assert!(set.get("none-alg").is_none());
        }
    }

    #[test]
    fn apple_shaped_policy_narrows_the_generic_nine() {
        // RS512 on RSA: fine generically, refused by Apple's two-algorithm set —
        // proof the admitted set is a parameter and never a union (04a).
        let rs512 = json!({
            "kty": "RSA", "kid": "rs512", "alg": "RS512",
            "n": rsa_jwk()["n"], "e": "AQAB"
        });
        let generic = VerificationKeySet::from_jwks("p", &json!({"keys": [rs512.clone()]}), NINE)
            .expect("generic set builds");
        assert!(generic.get("rs512").is_some());

        let apple = VerificationKeySet::from_jwks("p", &json!({"keys": [rs512]}), APPLE_SHAPED)
            .expect("apple-shaped set builds");
        assert!(
            apple.get("rs512").is_none(),
            "Apple's policy must stay narrower than the union"
        );
    }

    #[test]
    fn duplicate_kid_resolves_the_eligible_entry_regardless_of_order() {
        let mut ineligible = rsa_jwk();
        ineligible["kid"] = json!("shared-kid");
        ineligible["use"] = json!("enc");

        let eligible = json!({
            "kty": "RSA", "kid": "shared-kid", "alg": "RS256", "use": "sig",
            "n": rsa_jwk()["n"], "e": "AQAB"
        });

        let enc_first = VerificationKeySet::from_jwks(
            "p",
            &json!({"keys": [ineligible.clone(), eligible.clone()]}),
            NINE,
        )
        .expect("mixed duplicates resolve");
        let sig_first =
            VerificationKeySet::from_jwks("p", &json!({"keys": [eligible, ineligible]}), NINE)
                .expect("mirror image resolves identically");

        assert_eq!(
            enc_first.get("shared-kid").unwrap().algorithm(),
            Algorithm::RS256
        );
        assert_eq!(
            sig_first.get("shared-kid").unwrap().algorithm(),
            Algorithm::RS256
        );
        assert_eq!(
            enc_first.get("shared-kid").unwrap().algorithm(),
            sig_first.get("shared-kid").unwrap().algorithm(),
            "array order must decide nothing"
        );
    }

    #[test]
    fn two_eligible_entries_sharing_a_kid_are_an_ambiguity_error() {
        let a = json!({
            "kty": "RSA", "kid": "twin", "alg": "RS256", "use": "sig",
            "n": rsa_jwk()["n"], "e": "AQAB"
        });
        let b = json!({
            "kty": "RSA", "kid": "twin", "alg": "PS256", "use": "sig",
            "n": rsa_jwk()["n"], "e": "AQAB"
        });

        let err = VerificationKeySet::from_jwks("p", &json!({"keys": [a, b]}), NINE)
            .expect_err("ambiguous selection must be a document error");
        assert!(
            err.to_string().contains("twin"),
            "the ambiguity names the contested kid: {err}"
        );
    }

    #[test]
    fn missing_keys_array_is_a_document_error_not_an_empty_set() {
        let err = VerificationKeySet::from_jwks("provider-x", &json!({"no_keys": true}), NINE)
            .expect_err("a body without 'keys' is malformed");
        assert!(
            matches!(err, Error::ProviderError { .. }),
            "malformed JWKS surfaces as a provider fault: {err:?}"
        );
    }

    #[test]
    fn absent_alg_infers_narrowly_across_all_supported_shapes() {
        // RSA → RS256.
        let mut rsa = rsa_jwk();
        rsa.as_object_mut().unwrap().remove("alg");
        rsa["kid"] = json!("rsa-noalg");

        // EC P-256 → ES256.
        let ec = json!({
            "kty": "EC", "kid": "ec-noalg", "use": "sig", "crv": "P-256",
            "x": "MKBCTNIcKUSDii11ySs3526iDZ8AiTo7Tu6KPAqv7D4",
            "y": "4Etl6SRW2YiLUrN5vfvVHuhp7x8PxltmWWlbbM4IFyM"
        });

        // OKP Ed25519 → EdDSA.
        let okp = json!({
            "kty": "OKP", "kid": "okp-noalg", "use": "sig", "crv": "Ed25519",
            "x": "11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURo"
        });

        let set = VerificationKeySet::from_jwks("p", &json!({"keys": [rsa, ec, okp]}), NINE)
            .expect("all three alg-less shapes infer");
        assert_eq!(set.get("rsa-noalg").unwrap().algorithm(), Algorithm::RS256);
        assert_eq!(set.get("ec-noalg").unwrap().algorithm(), Algorithm::ES256);
        assert_eq!(set.get("okp-noalg").unwrap().algorithm(), Algorithm::EdDSA);

        // OKP on a non-signature curve has NO arm to land in.
        let weird_okp = json!({
            "kty": "OKP", "kid": "okp-weird", "use": "sig", "crv": "X25519",
            "x": "11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURo"
        });
        let set = VerificationSetHelper::build(&json!({"keys": [weird_okp]}));
        assert_eq!(set.len(), 0, "X25519 OKP must not infer EdDSA");
    }

    /// Small local helper so the negative-space assertions read plainly.
    struct VerificationSetHelper;
    impl VerificationSetHelper {
        fn build(jwks: &serde_json::Value) -> VerificationKeySet {
            VerificationKeySet::from_jwks("p", jwks, NINE).expect("document shape is valid")
        }
    }

    #[test]
    fn inference_cannot_widen_an_admitted_set() {
        // An alg-less EC P-384 key infers ES384 — inside the generic nine but
        // outside Apple's set, so Apple must still refuse it.
        let ec384 = json!({
            "kty": "EC", "kid": "ec384-noalg", "use": "sig", "crv": "P-384",
            "x": "MKBCTNIcKUSDii11ySs3526iDZ8AiTo7Tu6KPAqv7D4",
            "y": "4Etl6SRW2YiLUrN5vfvVHuhp7x8PxltmWWlbbM4IFyM"
        });

        let generic = VerificationKeySet::from_jwks("p", &json!({"keys": [ec384.clone()]}), NINE)
            .expect("generic admits the inferred ES384");
        assert!(generic.get("ec384-noalg").is_some());

        let apple = VerificationKeySet::from_jwks("p", &json!({"keys": [ec384]}), APPLE_SHAPED)
            .expect("document remains well-formed");
        assert!(
            apple.get("ec384-noalg").is_none(),
            "inference must pass admission like any declared algorithm"
        );
    }

    #[test]
    fn undecodable_material_drops_only_its_own_entry() {
        let good = json!({
            "kty": "RSA", "kid": "good", "alg": "RS256", "use": "sig",
            "n": rsa_jwk()["n"], "e": "AQAB"
        });
        let bad_material = json!({
            "kty": "EC", "kid": "bad-material", "alg": "ES256", "use": "sig",
            "crv": "P-256", "x": "!!!not-base64!!!", "y": "likewise!!!"
        });

        let set = VerificationKeySet::from_jwks("p", &json!({"keys": [bad_material, good]}), NINE)
            .expect("one rotten entry does not sink the document");
        assert!(set.get("bad-material").is_none());
        assert!(set.get("good").is_some());
    }
}
