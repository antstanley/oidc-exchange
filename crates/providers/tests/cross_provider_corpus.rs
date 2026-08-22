//! Cross-provider key-selection corpus (threat-model contradiction C12).
//!
//! Every case in `oidc_exchange_test_utils::corpus` is served — byte-identical —
//! to both the generic OIDC validator and the Apple validator, and each
//! validator's disposition is recorded. The baseline below is the
//! **pre-consolidation record**: it captures what the two private `find_jwk`
//! copies plus their per-provider `alg` matches actually do today, drift and
//! all, so the `VerificationKeySet` consolidation is a deliberate superset
//! decision rather than an accident. When the consolidation lands, this file's
//! expectation table flips to uniform dispositions — that flip is the C12
//! closure evidence.
//!
//! Dispositions are observable without forging signatures: key selection
//! happens strictly before signature verification, so
//! - `SelectionRejected` — the validator errors before attempting a signature
//!   check (ineligible key, unknown/absent algorithm with no inference arm, …);
//! - `SelectionAccepted` — the validator reached signature verification, which
//!   fails on the corpus's deliberately unsigned tokens ("JWT validation
//!   failed: …").
//!
//! Two properly signed success cases prove the corpus can still say "yes": the
//! non-regression requirement that a `use: "sig"` entry verifies on both paths.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use jsonwebtoken::{encode as jwt_encode, Algorithm, EncodingKey, Header};
use oidc_exchange_adapters::oidc::OidcProvider;
use oidc_exchange_core::domain::{IdentityClaims, OidcProviderConfig};
use oidc_exchange_core::error::Error;
use oidc_exchange_core::ports::IdentityProvider;
use oidc_exchange_providers::apple::AppleProvider;
use oidc_exchange_test_utils::corpus::{self, Case};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const OIDC_CLIENT_ID: &str = "corpus-client";
const APPLE_CLIENT_ID: &str = "com.example.corpus";
const APPLE_ISSUER: &str = "https://appleid.apple.com";

/// What a validator did with a corpus case's key selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Disposition {
    /// The validator resolved the key and reached signature verification.
    SelectionAccepted,
    /// The validator refused the key before any signature check.
    SelectionRejected,
}

/// The recorded pre-consolidation baseline: what each validator does with each
/// case **today**. Disagreements between the two columns are the C12 evidence.
const BASELINE: &[(Case, Disposition, Disposition, &str)] = &[
    // Absent `alg` on an RSA key: the generic path infers RS256, Apple refuses.
    // This is C12's headline divergence.
    (
        Case::RsaSigAbsentAlg,
        Disposition::SelectionAccepted,
        Disposition::SelectionRejected,
        "absent alg on sig RSA: OIDC infers, Apple refuses",
    ),
    (
        Case::RsaEncAbsentAlg,
        Disposition::SelectionAccepted,
        Disposition::SelectionRejected,
        "absent alg on enc RSA: same inference divergence, wrong purpose ignored",
    ),
    // An encryption-purpose key with an admitted signing alg: BOTH validators
    // accept it — neither consults `use`. The reproduced vector, agreeing for
    // the wrong reason.
    (
        Case::RsaEncRs256,
        Disposition::SelectionAccepted,
        Disposition::SelectionAccepted,
        "use=enc with RS256: neither validator filters purpose",
    ),
    (
        Case::RsaKeyOpsEncryptWrap,
        Disposition::SelectionAccepted,
        Disposition::SelectionAccepted,
        "key_ops without verify: neither validator filters operations",
    ),
    (
        Case::RsaKeyOpsEncryptOnly,
        Disposition::SelectionAccepted,
        Disposition::SelectionAccepted,
        "key_ops=encrypt: neither validator filters operations",
    ),
    // ES256 declared on an RSA key: both accept — the alg match arms never
    // check the declared algorithm against the key's family.
    (
        Case::RsaAlgEs256,
        Disposition::SelectionAccepted,
        Disposition::SelectionAccepted,
        "alg/kty inconsistency: neither validator cross-checks",
    ),
    (
        Case::RsaAlgRsaOaep,
        Disposition::SelectionAccepted,
        Disposition::SelectionRejected,
        "RSA-OAEP: OIDC's inference fallback rescues an unknown alg, Apple refuses",
    ),
    // EC P-256 with `use: enc` and no alg: OIDC infers ES256, Apple refuses.
    (
        Case::EcEncEs256,
        Disposition::SelectionAccepted,
        Disposition::SelectionRejected,
        "alg-less enc EC: inference divergence again",
    ),
    // Duplicate kid, ineligible entry first: OIDC's first-match-wins lands on
    // the enc key and infers RS256; Apple lands on it and refuses the alg.
    // The same JWKS changes verdict with array order on the Apple path.
    (
        Case::DuplicateKidEncFirst,
        Disposition::SelectionAccepted,
        Disposition::SelectionRejected,
        "duplicate kid, enc first: order-dependent on Apple, inference on OIDC",
    ),
    (
        Case::DuplicateKidSigFirst,
        Disposition::SelectionAccepted,
        Disposition::SelectionAccepted,
        "duplicate kid, sig first: both accept, proving the order dependence above",
    ),
    (
        Case::OctKey,
        Disposition::SelectionRejected,
        Disposition::SelectionRejected,
        "oct key: both refuse (OIDC inference has no oct arm; Apple sees no alg)",
    ),
    // `alg: "none"`: OIDC's inference treats the unrecognised value as absent
    // and resolves RS256; Apple refuses. The exact unknown-vs-absent conflation
    // the source spec calls out.
    (
        Case::AlgNone,
        Disposition::SelectionAccepted,
        Disposition::SelectionRejected,
        "alg=none: OIDC conflates unknown with absent, Apple refuses",
    ),
];

fn b64url(value: &serde_json::Value) -> String {
    URL_SAFE_NO_PAD.encode(serde_json::to_vec(value).expect("claims serialize"))
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is sane")
        .as_secs()
}

/// A deliberately unsigned three-segment token whose header carries `kid`.
///
/// Selection happens before signature verification, so the signature segment is
/// never reached on rejection paths; on acceptance paths verification fails with
/// the recognisable "JWT validation failed" prefix. The claims are valid for the
/// target provider so only the signature can fail once selection accepts.
fn unsigned_token(kid: &str, issuer: &str, audience: &str) -> String {
    let header = json!({ "alg": "RS256", "kid": kid });
    let claims = json!({
        "iss": issuer,
        "aud": audience,
        "sub": "corpus-subject",
        "iat": now_epoch(),
        "exp": now_epoch() + 3600,
    });
    format!("{}.{}.{}", b64url(&header), b64url(&claims), "bm90LWEtc2ln")
}

/// A properly signed token for the non-regression success cases (`is_rsa`
/// selects RS256 over ES256 to match the embedded corpus key).
fn signed_token(pem: &[u8], is_rsa: bool, kid: &str, issuer: &str, audience: &str) -> String {
    let claims = json!({
        "iss": issuer,
        "aud": audience,
        "sub": "corpus-subject",
        "iat": now_epoch(),
        "exp": now_epoch() + 3600,
    });
    let mut header = if is_rsa {
        Header::new(Algorithm::RS256)
    } else {
        Header::new(Algorithm::ES256)
    };
    header.kid = Some(kid.to_string());
    let encoding_key = if is_rsa {
        EncodingKey::from_rsa_pem(pem).expect("corpus RSA PEM parses")
    } else {
        EncodingKey::from_ec_pem(pem).expect("corpus EC PEM parses")
    };
    jwt_encode(&header, &claims, &encoding_key).expect("signing with the corpus key works")
}

/// Serve `jwks` at `/jwks.json` and return the server URI.
async fn serve_jwks(jwks: &serde_json::Value) -> String {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(jwks))
        .mount(&server)
        .await;
    server.uri()
}

fn oidc_config(server_uri: &str) -> OidcProviderConfig {
    OidcProviderConfig {
        provider_id: "corpus-oidc".into(),
        issuer: server_uri.to_string(),
        client_id: OIDC_CLIENT_ID.into(),
        client_secret: None,
        jwks_uri: Some(format!("{server_uri}/jwks.json")),
        token_endpoint: Some(format!("{server_uri}/token")),
        revocation_endpoint: None,
        scopes: vec!["openid".into()],
        additional_params: HashMap::new(),
    }
}

/// Build the Apple provider against a mock JWKS origin. `AppleProvider` takes
/// its private key as a filesystem path, so the corpus PEM goes through a temp
/// file whose guard outlives the provider under test.
async fn apple_provider(server_uri: &str) -> (AppleProvider, tempfile::TempPath) {
    let pem_file = tempfile::NamedTempFile::new().expect("temp file for corpus PEM");
    std::fs::write(pem_file.path(), corpus::EC_PRIVATE_PEM).expect("PEM write");
    let pem_guard = pem_file.into_temp_path();

    let mut raw = HashMap::new();
    raw.insert(
        "client_id".to_string(),
        toml::Value::String(APPLE_CLIENT_ID.into()),
    );
    raw.insert(
        "team_id".to_string(),
        toml::Value::String("CORPUSTEAM".into()),
    );
    raw.insert(
        "key_id".to_string(),
        toml::Value::String("corpus-key".into()),
    );
    raw.insert(
        "private_key_path".to_string(),
        toml::Value::String(pem_guard.display().to_string()),
    );
    raw.insert(
        "token_endpoint".to_string(),
        toml::Value::String(format!("{server_uri}/token")),
    );
    raw.insert(
        "jwks_uri".to_string(),
        toml::Value::String(format!("{server_uri}/jwks.json")),
    );

    let provider = AppleProvider::from_config(&raw)
        .await
        .expect("Apple provider builds against the mock JWKS");
    (provider, pem_guard)
}

/// The kid a validator must look up for a case.
fn kid_for_case(case: Case) -> &'static str {
    match case {
        Case::OctKey => "corpus-oct-key",
        Case::DuplicateKidEncFirst | Case::DuplicateKidSigFirst => corpus::DUPLICATE_KID,
        Case::EcEncEs256 => corpus::EC_KID,
        _ => corpus::RSA_KID,
    }
}

/// Classify a validation outcome into a corpus disposition.
///
/// Reaching signature verification always fails with the "JWT validation
/// failed" prefix on an unsigned token; every earlier refusal is a different
/// message. The classification therefore observes exactly the boundary the
/// corpus cares about: did the validator accept this key as a candidate?
fn classify(outcome: Result<IdentityClaims, Error>) -> Disposition {
    match outcome {
        Err(e) if e.to_string().contains("JWT validation failed") => Disposition::SelectionAccepted,
        Err(_) => Disposition::SelectionRejected,
        Ok(_) => Disposition::SelectionAccepted,
    }
}

async fn oidc_disposition(case: Case) -> Disposition {
    let jwks = corpus::jwks_for_case(case);
    let server_uri = serve_jwks(&jwks).await;
    let provider = OidcProvider::from_config("corpus-oidc", &oidc_config(&server_uri))
        .await
        .expect("OIDC provider builds against the mock JWKS");

    let token = unsigned_token(kid_for_case(case), &server_uri, OIDC_CLIENT_ID);
    classify(provider.validate_id_token(&token).await)
}

async fn apple_disposition(case: Case) -> Disposition {
    let jwks = corpus::jwks_for_case(case);
    let server_uri = serve_jwks(&jwks).await;
    let (provider, _pem_guard) = apple_provider(&server_uri).await;

    let token = unsigned_token(kid_for_case(case), APPLE_ISSUER, APPLE_CLIENT_ID);
    classify(provider.validate_id_token(&token).await)
}

/// THE C12 BASELINE: run every corpus case through both validators and assert
/// the recorded pre-consolidation dispositions, drift included. Changing a
/// validator's behaviour without updating this table fails here first.
#[tokio::test]
async fn baseline_records_current_dispositions_for_every_corpus_case() {
    let mut disagreements = 0usize;

    for (case, expected_oidc, expected_apple, why) in BASELINE {
        let oidc = oidc_disposition(*case).await;
        let apple = apple_disposition(*case).await;

        assert_eq!(
            oidc, *expected_oidc,
            "OIDC disposition drifted from the baseline for {case:?} ({why})"
        );
        assert_eq!(
            apple, *expected_apple,
            "Apple disposition drifted from the baseline for {case:?} ({why})"
        );
        if oidc != apple {
            disagreements += 1;
        }
    }

    // The point of the baseline, made explicit: the two validators disagree on
    // six of the twelve cases today, which records contradiction C12 as
    // evidence rather than assuming it either way.
    assert_eq!(
        disagreements, 6,
        "the recorded number of OIDC/Apple disagreements is part of the baseline"
    );
}

/// Non-regression: a real `use: sig` RSA key with `alg: RS256` verifies a
/// properly signed token on the generic path.
#[tokio::test]
async fn rsa_sig_key_verifies_on_the_oidc_path() {
    let jwks = json!({ "keys": [corpus::rsa_sig_entry(corpus::RSA_KID)] });
    let server_uri = serve_jwks(&jwks).await;
    let provider = OidcProvider::from_config("corpus-oidc", &oidc_config(&server_uri))
        .await
        .expect("provider builds");

    let token = signed_token(
        corpus::RSA_PRIVATE_PEM.as_bytes(),
        true,
        corpus::RSA_KID,
        &server_uri,
        OIDC_CLIENT_ID,
    );

    let claims = provider
        .validate_id_token(&token)
        .await
        .expect("a use:sig RSA key must still verify (non-regression)");
    assert_eq!(claims.subject, "corpus-subject");
}

/// Non-regression: a real `use: sig` P-256 key verifies a properly signed
/// token on the Apple path — the shape Apple's own JWKS actually ships.
#[tokio::test]
async fn ec_sig_key_verifies_on_the_apple_path() {
    let jwks = json!({ "keys": [corpus::ec_sig_entry(corpus::EC_KID)] });
    let server_uri = serve_jwks(&jwks).await;
    let (provider, _pem_guard) = apple_provider(&server_uri).await;

    let token = signed_token(
        corpus::EC_PRIVATE_PEM.as_bytes(),
        false,
        corpus::EC_KID,
        APPLE_ISSUER,
        APPLE_CLIENT_ID,
    );

    let claims = provider
        .validate_id_token(&token)
        .await
        .expect("a use:sig P-256 key must still verify (non-regression)");
    assert_eq!(claims.subject, "corpus-subject");
}
