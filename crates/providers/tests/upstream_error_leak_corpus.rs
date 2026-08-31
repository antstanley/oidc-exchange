//! Upstream-error leak corpus (plan task 07): provider non-2xx paths under a capturing
//! subscriber with span close events enabled.
//!
//! The upstream redactor (`shared::upstream::error_detail`) is the single audited point
//! where upstream bytes become loggable text. These tests drive the *real* provider
//! paths against hostile mock upstreams that echo submitted credentials back — raw,
//! percent-encoded, JSON-wrapped, and as bare compact JWS — and assert two things:
//!
//! 1. The resulting `ProviderError` `Display` (which the server's error mapping logs in
//!    full) carries only `[REDACTED]` markers, never the sentinel material.
//! 2. Nothing the provider path itself emits to telemetry while handling the failure
//!    renders any sentinel either.
//!
//! Sentinels are distinct per credential class: the token being revoked, the
//! authorization code at the exchange boundary, and Apple's freshly generated client
//! assertion (captured live from the wire, since it is minted per call).

use std::collections::HashMap;

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use oidc_exchange_core::config::HttpsUrl;
use oidc_exchange_core::error::Error;
use oidc_exchange_core::ports::IdentityProvider;
use oidc_exchange_test_utils::telemetry::{
    assert_absent_plain_and_encoded, install_span_capture, SharedBuffer,
};

/// The token presented for revocation.
const REVOKE_TOKEN_SENTINEL: &str = "SENTINEL-REVOKE-TOKEN-VALUE";
/// The authorization code presented at the exchange boundary.
const CODE_SENTINEL: &str = "SENTINEL-EXCHANGE-CODE-VALUE";

fn echo_body() -> String {
    format!(
        "upstream rejected token={REVOKE_TOKEN_SENTINEL} \
         &echo-encoded=token%3D1%2F%2F{REVOKE_TOKEN_SENTINEL} \
         &json={{\"token\":\"{REVOKE_TOKEN_SENTINEL}\"}}"
    )
}

// ---------------------------------------------------------------------------
// OIDC provider revocation
// ---------------------------------------------------------------------------

async fn oidc_provider(
    revocation_endpoint: Option<HttpsUrl>,
) -> oidc_exchange_adapters::oidc::OidcProvider {
    let config = oidc_exchange_core::domain::OidcProviderConfig {
        provider_id: "corpus-oidc".to_string(),
        issuer: HttpsUrl::parse("https://issuer.example.com").expect("valid url"),
        client_id: "corpus-client-id".to_string(),
        client_secret: Some(oidc_exchange_core::Secret::new(
            "sentinel-configured-client-secret".to_string(),
        )),
        jwks_uri: Some(HttpsUrl::parse("https://issuer.example.com/jwks.json").expect("valid url")),
        token_endpoint: Some(
            HttpsUrl::parse("https://issuer.example.com/token").expect("valid url"),
        ),
        revocation_endpoint,
        endpoint_origins: Vec::new(),
        email_verification: oidc_exchange_core::domain::EmailVerification::default(),
        scopes: Vec::new(),
        additional_params: HashMap::new(),
    };
    oidc_exchange_adapters::oidc::OidcProvider::from_config("corpus-oidc", &config)
        .await
        .expect("build OIDC provider from explicit endpoints")
}

#[tokio::test]
async fn oidc_revoke_non_2xx_echo_leaks_nothing_into_error_or_telemetry() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/revoke"))
        .respond_with(ResponseTemplate::new(400).set_body_string(echo_body()))
        .expect(1)
        .mount(&server)
        .await;

    let capture = install_span_capture(SharedBuffer::default());
    tracing::info!(target: "oidc_exchange_corpus", "corpus-marker: oidc revoke start");
    let provider = oidc_provider(Some(
        HttpsUrl::parse_for_test(format!("{}/revoke", server.uri())).expect("wiremock url"),
    ))
    .await;

    let err = provider
        .revoke_token(REVOKE_TOKEN_SENTINEL)
        .await
        .expect_err("a 400 echo must fail");

    match &err {
        Error::ProviderError { provider, detail } => {
            assert_eq!(provider, "corpus-oidc");
            assert!(
                detail.contains("[REDACTED]"),
                "the redaction marker must be present, got {detail:?}"
            );
        }
        other => panic!("expected ProviderError, got {other:?}"),
    }

    // The error's Display is exactly what the server logs; it must be clean too.
    let message = err.to_string();
    assert!(
        !message.contains(REVOKE_TOKEN_SENTINEL),
        "revoked token must never reach the error Display, got {message:?}"
    );

    let rendered = capture.rendered();
    assert!(
        rendered.contains("corpus-marker"),
        "the capture must be live for the absence claims below"
    );
    assert_absent_plain_and_encoded(&rendered, REVOKE_TOKEN_SENTINEL);
    assert_absent_plain_and_encoded(&rendered, "sentinel-configured-client-secret");
}

/// A conformant RFC 6749 structured error stays visible (operators need the class) while
/// echoed credentials inside `error_description` are masked.
#[tokio::test]
async fn oidc_revoke_structured_error_stays_visible_but_masked() {
    let server = MockServer::start().await;
    let body = format!(
        r#"{{"error":"invalid_grant","error_description":"rejected token={REVOKE_TOKEN_SENTINEL}"}}"#
    );
    Mock::given(method("POST"))
        .and(path("/revoke"))
        .respond_with(ResponseTemplate::new(400).set_body_string(body))
        .mount(&server)
        .await;

    let capture = install_span_capture(SharedBuffer::default());
    let provider = oidc_provider(Some(
        HttpsUrl::parse_for_test(format!("{}/revoke", server.uri())).expect("wiremock url"),
    ))
    .await;

    let err = provider
        .revoke_token(REVOKE_TOKEN_SENTINEL)
        .await
        .expect_err("structured 400 must fail");
    let message = err.to_string();

    assert!(
        message.contains("invalid_grant"),
        "the RFC 6749 error class must stay visible to operators, got {message:?}"
    );
    assert!(
        !message.contains(REVOKE_TOKEN_SENTINEL),
        "the echoed token inside error_description must be masked, got {message:?}"
    );
    assert_absent_plain_and_encoded(&capture.rendered(), REVOKE_TOKEN_SENTINEL);
}

// ---------------------------------------------------------------------------
// Shared token-endpoint exchange (OIDC adapter's exchange_code path)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn exchange_non_2xx_leaks_no_code_or_secret() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(401).set_body_string(format!(
            "denied code={CODE_SENTINEL} encoded=code%3D{CODE_SENTINEL}"
        )))
        .mount(&server)
        .await;

    let config = oidc_exchange_core::domain::OidcProviderConfig {
        provider_id: "corpus-exchange".to_string(),
        issuer: HttpsUrl::parse("https://issuer.example.com").expect("valid url"),
        client_id: "corpus-client-id".to_string(),
        client_secret: Some(oidc_exchange_core::Secret::new(
            "sentinel-exchange-client-secret".to_string(),
        )),
        jwks_uri: Some(HttpsUrl::parse("https://issuer.example.com/jwks.json").expect("valid url")),
        token_endpoint: Some(
            HttpsUrl::parse_for_test(format!("{}/token", server.uri())).expect("wiremock url"),
        ),
        revocation_endpoint: None,
        endpoint_origins: Vec::new(),
        email_verification: oidc_exchange_core::domain::EmailVerification::default(),
        scopes: Vec::new(),
        additional_params: HashMap::new(),
    };
    let provider =
        oidc_exchange_adapters::oidc::OidcProvider::from_config("corpus-exchange", &config)
            .await
            .expect("build provider");

    let capture = install_span_capture(SharedBuffer::default());
    let err = provider
        .exchange_code(CODE_SENTINEL, "https://client.example.com/callback")
        .await
        .expect_err("a 401 exchange must fail");

    let message = err.to_string();
    assert!(
        !message.contains(CODE_SENTINEL),
        "authorization code must never reach the error Display, got {message:?}"
    );
    assert!(
        !message.contains("sentinel-exchange-client-secret"),
        "client secret must never reach the error Display, got {message:?}"
    );

    let rendered = capture.rendered();
    assert_absent_plain_and_encoded(&rendered, CODE_SENTINEL);
    assert_absent_plain_and_encoded(&rendered, "sentinel-exchange-client-secret");
}

// ---------------------------------------------------------------------------
// Apple provider revocation — assertion captured live off the wire
// ---------------------------------------------------------------------------

#[tokio::test]
async fn apple_revoke_non_2xx_leaks_no_token_or_generated_assertion() {
    use p256::elliptic_curve::Generate;
    use p256::pkcs8::EncodePrivateKey;

    let server = MockServer::start().await;
    let uri = server.uri();

    // Phase 0: materialize a throwaway ES256 key so `from_config` can read a real .p8.
    let signing_key = p256::ecdsa::SigningKey::generate();
    let pem = signing_key
        .to_pkcs8_pem(p256::pkcs8::LineEnding::LF)
        .expect("PEM encoding should work");
    let key_dir = tempfile::TempDir::new().expect("temp dir for p8 key");
    let key_path = key_dir.path().join("corpus.p8");
    std::fs::write(&key_path, pem.as_bytes()).expect("write .p8 key file");

    let mut config_map: HashMap<String, toml::Value> = HashMap::new();
    config_map.insert(
        "client_id".to_string(),
        toml::Value::String("com.example.corpus".to_string()),
    );
    config_map.insert(
        "team_id".to_string(),
        toml::Value::String("CORPUSTEAM1".to_string()),
    );
    config_map.insert(
        "key_id".to_string(),
        toml::Value::String("corpus-key-1".to_string()),
    );
    config_map.insert(
        "private_key_path".to_string(),
        toml::Value::String(key_path.to_string_lossy().into_owned()),
    );
    config_map.insert(
        "token_endpoint".to_string(),
        toml::Value::String(format!("{uri}/auth/token")),
    );
    config_map.insert(
        "jwks_uri".to_string(),
        toml::Value::String(format!("{uri}/auth/keys")),
    );
    config_map.insert(
        "revocation_endpoint".to_string(),
        toml::Value::String(format!("{uri}/auth/revoke")),
    );

    // Phase 1: one failing revoke whose request we inspect to capture the assertion
    // Apple's provider just signed — it is minted per call, so it cannot be a constant.
    Mock::given(method("POST"))
        .and(path("/auth/revoke"))
        .respond_with(ResponseTemplate::new(400).set_body_string("phase-1 rejection"))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    let capture = install_span_capture(SharedBuffer::default());
    tracing::info!(target: "oidc_exchange_corpus", "corpus-marker: apple revoke start");
    let provider = {
        let pem = std::fs::read(&key_path).expect("read corpus signing key");
        let signing_key =
            jsonwebtoken::EncodingKey::from_ec_pem(&pem).expect("corpus signing key parses");
        oidc_exchange_providers::apple::AppleProvider::new_for_test(
            "com.example.corpus".to_string(),
            "CORPUSTEAM1".to_string(),
            "corpus-key-1".to_string(),
            signing_key,
            HttpsUrl::parse_for_test(format!("{uri}/auth/token")).expect("wiremock url"),
            HttpsUrl::parse_for_test(format!("{uri}/auth/keys")).expect("wiremock url"),
            Some(HttpsUrl::parse_for_test(format!("{uri}/auth/revoke")).expect("wiremock url")),
        )
    };

    assert!(
        provider.revoke_token(REVOKE_TOKEN_SENTINEL).await.is_err(),
        "the phase-1 400 must fail"
    );
    let requests = server
        .received_requests()
        .await
        .expect("wiremock request recording must be enabled");
    let form = String::from_utf8(requests[0].body.clone()).expect("form body is UTF-8");
    let assertion: String = form
        .split('&')
        .find_map(|pair| pair.strip_prefix("client_secret="))
        .map(String::from)
        .filter(|v| !v.is_empty())
        .expect("the failing request must carry a generated client assertion");

    // Phase 2: echo everything back — the token (raw + percent-encoded), the assertion,
    // and a bare JWS-shaped run outside any key/value context.
    let echo = format!(
        "token={REVOKE_TOKEN_SENTINEL}&client_secret={assertion}\
         &encoded=token%3D1%2F%2F{REVOKE_TOKEN_SENTINEL} \
         stray-jws={assertion}"
    );
    Mock::given(method("POST"))
        .and(path("/auth/revoke"))
        .respond_with(ResponseTemplate::new(400).set_body_string(echo))
        .mount(&server)
        .await;

    let err = provider
        .revoke_token(REVOKE_TOKEN_SENTINEL)
        .await
        .expect_err("a 400 echo must fail");
    let message = err.to_string();

    assert!(
        !message.contains(REVOKE_TOKEN_SENTINEL),
        "revoked token (raw or decoded) must never reach the Display, got {message:?}"
    );
    assert!(
        !message.contains(&assertion),
        "the generated client assertion must never reach the Display, got {message:?}"
    );
    assert!(
        message.contains("[REDACTED]"),
        "redaction markers must be present, got {message:?}"
    );

    let rendered = capture.rendered();
    assert!(rendered.contains("corpus-marker"));
    assert_absent_plain_and_encoded(&rendered, REVOKE_TOKEN_SENTINEL);
    assert_absent_plain_and_encoded(&rendered, &assertion);
}
