//! Cross-provider key-selection corpus (threat-model contradiction C12).
//!
//! Every case in `oidc_exchange_test_utils::corpus` is served — byte-identical —
//! to both the generic OIDC validator and the Apple validator, and each
//! validator's disposition is asserted. This file is the **post-consolidation
//! record**: both validators now select keys through the shared
//! `VerificationKeySet` constructor (each with its own admitted-algorithm
//! policy), so their selection eligibility agrees on every case. The
//! pre-consolidation baseline this table replaced showed the validators
//! disagreeing on 6 of 12 cases; zero disagreements here is the C12 closure
//! evidence.
//!
//! Dispositions are observable without forging signatures: key selection
//! happens strictly before signature verification, so
//! - `SelectionRejected` — the validator errors before attempting a signature
//!   check (ineligible key, unknown/absent algorithm with no inference arm, or
//!   a plain `kid` miss);
//! - `SelectionAccepted` — the validator resolved an eligible key and reached
//!   signature verification, which fails on the corpus's deliberately unsigned
//!   tokens ("JWT validation failed: …"). Acceptance here means "this key is a
//!   legitimate verification candidate", not "the token is valid" — the corpus
//!   tokens are never validly signed by construction.
//!
//! Two properly signed success cases prove the corpus can still say "yes": the
//! non-regression requirement that a `use: "sig"` entry verifies on both paths.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use jsonwebtoken::{encode as jwt_encode, Algorithm, EncodingKey, Header};
use oidc_exchange_adapters::oidc::OidcProvider;
use oidc_exchange_core::config::HttpsUrl;
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

use Disposition::SelectionAccepted as ACCEPT;
use Disposition::SelectionRejected as REJECT;

/// The post-consolidation record: what both validators do with each case now
/// that selection lives in the shared `VerificationKeySet` constructor. The two
/// disposition columns are asserted equal case by case — that equality is the
/// C12 answer.
///
/// Per-case reasoning (constructor rules in `shared::keys`):
/// - `RsaSigAbsentAlg` — eligible on both: absent `alg` infers RS256 from the
///   RSA material, inside both admitted sets. The Azure-AD compatibility case.
/// - `RsaEncAbsentAlg`, `RsaEncRs256`, `EcEncEs256` — `use: enc` is dropped.
/// - `RsaKeyOpsEncryptWrap`, `RsaKeyOpsEncryptOnly` — `key_ops` without
///   `verify` is dropped.
/// - `RsaAlgEs256` — declared `alg` inconsistent with `kty` is dropped.
/// - `RsaAlgRsaOaep`, `AlgNone` — unknown *declared* algorithms are dropped
///   outright; they are never treated as absent, so inference never rescues
///   them (the exact conflation the pre-consolidation OIDC path had).
/// - `DuplicateKid{Enc,Sig}First` — the eligible `sig` entry resolves in both
///   array orders; order decides nothing.
/// - `OctKey` — symmetric material never verifies asymmetric assertions.
const POST_CONSOLIDATION: &[(Case, Disposition, &str)] = &[
    (
        Case::RsaSigAbsentAlg,
        ACCEPT,
        "absent alg on sig RSA: narrowed inference resolves RS256 on both paths",
    ),
    (Case::RsaEncAbsentAlg, REJECT, "use=enc is dropped"),
    (
        Case::RsaEncRs256,
        REJECT,
        "use=enc is dropped despite the admitted alg",
    ),
    (
        Case::RsaKeyOpsEncryptWrap,
        REJECT,
        "key_ops without verify is dropped",
    ),
    (
        Case::RsaKeyOpsEncryptOnly,
        REJECT,
        "key_ops=encrypt is dropped",
    ),
    (
        Case::RsaAlgEs256,
        REJECT,
        "ES256 declared on RSA material is inconsistent",
    ),
    (
        Case::RsaAlgRsaOaep,
        REJECT,
        "RSA-OAEP is an unknown declared alg: dropped, never inferred around",
    ),
    (Case::EcEncEs256, REJECT, "use=enc is dropped"),
    (
        Case::DuplicateKidEncFirst,
        ACCEPT,
        "ineligible-first order still resolves the eligible sig entry",
    ),
    (
        Case::DuplicateKidSigFirst,
        ACCEPT,
        "mirror order resolves identically: order independence proven",
    ),
    (Case::OctKey, REJECT, "symmetric keys are never candidates"),
    (
        Case::AlgNone,
        REJECT,
        "alg=none is an unknown declared alg: dropped on both paths",
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
        issuer: HttpsUrl::parse_for_test(server_uri).expect("wiremock url"),
        client_id: OIDC_CLIENT_ID.into(),
        client_secret: None,
        jwks_uri: Some(
            HttpsUrl::parse_for_test(format!("{server_uri}/jwks.json")).expect("wiremock url"),
        ),
        token_endpoint: Some(
            HttpsUrl::parse_for_test(format!("{server_uri}/token")).expect("wiremock url"),
        ),
        revocation_endpoint: None,
        endpoint_origins: Vec::new(),
        email_verification: oidc_exchange_core::domain::EmailVerification::default(),
        scopes: vec!["openid".into()],
        additional_params: HashMap::new(),
    }
}

/// Build the Apple provider against a mock JWKS origin, through the hidden
/// test seam: the strict `from_config` HTTPS validation rightly refuses
/// wiremock's plain-HTTP endpoints, so the corpus constructs the provider
/// directly from the corpus PEM.
async fn apple_provider(server_uri: &str) -> AppleProvider {
    let signing_key = jsonwebtoken::EncodingKey::from_ec_pem(corpus::EC_PRIVATE_PEM.as_bytes())
        .expect("corpus EC PEM parses");

    AppleProvider::new_for_test(
        APPLE_CLIENT_ID.into(),
        "CORPUSTEAM".into(),
        "corpus-key".into(),
        signing_key,
        HttpsUrl::parse_for_test(format!("{server_uri}/token")).expect("wiremock url"),
        HttpsUrl::parse_for_test(format!("{server_uri}/jwks.json")).expect("wiremock url"),
        None,
    )
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
    let provider = apple_provider(&server_uri).await;

    let token = unsigned_token(kid_for_case(case), APPLE_ISSUER, APPLE_CLIENT_ID);
    classify(provider.validate_id_token(&token).await)
}

/// THE C12 CLOSURE: run every corpus case through both validators and assert
/// that selection eligibility now agrees everywhere. The pre-consolidation
/// baseline disagreed on 6 of 12 cases; this table asserts 0.
#[tokio::test]
async fn both_validators_agree_on_selection_for_every_corpus_case() {
    let mut disagreements = 0usize;

    for (case, expected, why) in POST_CONSOLIDATION {
        let oidc = oidc_disposition(*case).await;
        let apple = apple_disposition(*case).await;

        assert_eq!(
            oidc, *expected,
            "OIDC disposition drifted from the consolidated rule for {case:?} ({why})"
        );
        assert_eq!(
            apple, *expected,
            "Apple disposition drifted from the consolidated rule for {case:?} ({why})"
        );
        if oidc != apple {
            disagreements += 1;
        }
    }

    assert_eq!(
        disagreements, 0,
        "C12 is closed only while the two validators agree on every case"
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

/// Non-regression: a real `use: sig` P-256 key with `alg: ES256` verifies a
/// properly signed token on the Apple path — the shape Apple's own JWKS ships.
#[tokio::test]
async fn ec_sig_key_verifies_on_the_apple_path() {
    let jwks = json!({ "keys": [corpus::ec_sig_entry(corpus::EC_KID)] });
    let server_uri = serve_jwks(&jwks).await;
    let provider = apple_provider(&server_uri).await;

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

/// Non-regression on the second path: the same RSA `use: sig` key also verifies
/// through Apple's validator, because RS256 is in Apple's admitted set — the
/// corpus fixtures are identical across paths, and so is the outcome.
#[tokio::test]
async fn rsa_sig_key_verifies_on_the_apple_path() {
    let jwks = json!({ "keys": [corpus::rsa_sig_entry(corpus::RSA_KID)] });
    let server_uri = serve_jwks(&jwks).await;
    let provider = apple_provider(&server_uri).await;

    let token = signed_token(
        corpus::RSA_PRIVATE_PEM.as_bytes(),
        true,
        corpus::RSA_KID,
        APPLE_ISSUER,
        APPLE_CLIENT_ID,
    );

    let claims = provider
        .validate_id_token(&token)
        .await
        .expect("RS256 stays admitted for Apple after consolidation");
    assert_eq!(claims.subject, "corpus-subject");
}
