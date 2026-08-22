//! Focused tests for the core's assertion binding (`service::assertion`):
//! the direct ID-token grant's nonce consumption and every shared control —
//! lifetime ceiling, `azp`, applicable `at_hash`, and single-use replay
//! prevention — on both exchange paths, plus audit classification and typed
//! store-failure propagation.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use async_trait::async_trait;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::Utc;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use oidc_exchange_core::config::AppConfig;
use oidc_exchange_core::domain::{
    AuditEventType, AuditOutcome, AuditSeverity, IdentityClaims, Session,
};
use oidc_exchange_core::error::{Error, Result};
use oidc_exchange_core::ports::{IdentityProvider, SessionRepository, UserRepository};
use oidc_exchange_core::service::exchange::ExchangeRequest;
use oidc_exchange_core::service::{assertion, AppService};
use oidc_exchange_test_utils::{
    MockAuditLog, MockIdentityProvider, MockKeyManager, MockRepository, MockUserSync,
    MOCK_CLIENT_ID,
};

const PROVIDER_ID: &str = "mock";

/// Deterministically unique `jti` values so two exchanges never share a
/// replay marker unless a test deliberately reuses one. nextest runs each
/// test in its own process, so per-test counters stay deterministic.
fn unique_jti() -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    format!("jti-{}", COUNTER.fetch_add(1, Ordering::SeqCst))
}

/// Raw claims every binding control accepts by default: an `exp` ten minutes
/// out (inside the default 1h ceiling) and a fresh `jti`.
fn base_raw() -> HashMap<String, Value> {
    let mut raw = HashMap::new();
    raw.insert("exp".to_string(), json!(Utc::now().timestamp() + 600));
    raw.insert("jti".to_string(), json!(unique_jti()));
    raw.insert("sub".to_string(), json!("binding-subject"));
    raw
}

/// Verified claims with `signing_alg` and raw claims chosen by the caller.
fn claims_for(signing_alg: &str, raw: HashMap<String, Value>) -> IdentityClaims {
    IdentityClaims {
        subject: "binding-subject".to_string(),
        email: Some("binding@example.com".to_string()),
        email_verified: Some(true),
        name: Some("Binding User".to_string()),
        is_private_email: None,
        signing_alg: signing_alg.to_string(),
        raw_claims: raw,
    }
}

fn make_config() -> AppConfig {
    AppConfig::default()
}

fn config_with(max_assertion_lifetime: &str) -> AppConfig {
    let mut config = make_config();
    config.grants.max_assertion_lifetime = max_assertion_lifetime.to_string();
    config
}

fn make_auditing_service(
    repo: MockRepository,
    provider: MockIdentityProvider,
    config: AppConfig,
) -> (AppService, MockAuditLog, MockRepository) {
    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    let provider_id = provider.provider_id().to_string();
    providers.insert(provider_id, Box::new(provider));

    let audit = MockAuditLog::new();
    let service = AppService::new(
        Box::new(repo.clone()),
        Box::new(repo.clone()),
        Box::new(MockKeyManager::new()),
        Box::new(audit.clone()),
        Box::new(MockUserSync::new()),
        providers,
        config,
    );
    (service, audit, repo)
}

/// Mint one nonce through a throwaway service over the same repo.
async fn mint_nonce_over(repo: &MockRepository) -> String {
    let (minter, _audit, _repo) = make_auditing_service(
        repo.clone(),
        MockIdentityProvider::new(PROVIDER_ID),
        make_config(),
    );
    minter.mint_nonce().await.expect("mint nonce").nonce
}

/// The storage key a minted nonce value lives under.
fn nonce_key(nonce: &str) -> String {
    format!("nonce:{}", hex::encode(Sha256::digest(nonce.as_bytes())))
}

/// The direct-grant request shape: an id_token field, no code.
fn direct_request() -> ExchangeRequest {
    ExchangeRequest {
        id_token: Some("header.assertion.signature".to_string()),
        provider: PROVIDER_ID.to_string(),
        ..Default::default()
    }
}

/// The code-grant request shape.
fn code_request() -> ExchangeRequest {
    ExchangeRequest {
        code: Some("auth-code".to_string()),
        redirect_uri: Some("https://app.test.com/callback".to_string()),
        provider: PROVIDER_ID.to_string(),
        ..Default::default()
    }
}

fn expect_invalid_grant(err: Error, needle: &str) {
    match err {
        Error::InvalidGrant { reason } => assert!(
            reason.contains(needle),
            "expected {needle:?} in the rejection reason, got {reason:?}"
        ),
        other => panic!("expected InvalidGrant, got: {other:?}"),
    }
}

/// Assemble an auditing direct-grant fixture: mints a fresh nonce over the
/// shared repo, pins provider claims built from `raw` (which must not carry a
/// nonce — the fixture supplies it), and attaches the optional provider
/// access token to the request. The provider handle comes back so tests can
/// re-pin claims (e.g. with a fresh nonce) between exchanges.
#[allow(clippy::type_complexity)]
async fn direct_fixture(
    signing_alg: &str,
    raw: HashMap<String, Value>,
    access_token: Option<&str>,
) -> (
    AppService,
    MockAuditLog,
    MockRepository,
    MockIdentityProvider,
    ExchangeRequest,
) {
    let repo = MockRepository::new();
    let nonce = mint_nonce_over(&repo).await;

    let mut raw = raw;
    assert!(
        raw.insert("nonce".to_string(), json!(nonce)).is_none(),
        "fixture callers must not pre-set the nonce"
    );

    let provider = MockIdentityProvider::new(PROVIDER_ID);
    provider.set_claims(claims_for(signing_alg, raw)).await;

    let mut request = direct_request();
    request.provider_access_token = access_token.map(str::to_string);

    let (svc, audit, repo) = make_auditing_service(repo, provider.clone(), make_config());
    (svc, audit, repo, provider, request)
}

// ---------------------------------------------------------------------------
// Nonce minting
// ---------------------------------------------------------------------------

/// A minted nonce has the documented wire size, reports the configured TTL,
/// and lands in the store only as its SHA-256 digest under the `nonce:` key.
#[tokio::test]
async fn mint_nonce_stores_only_the_digest_key() {
    let mut config = make_config();
    config.grants.nonce_ttl = "90s".to_string();
    let repo = MockRepository::new();
    let (svc, _audit, repo) =
        make_auditing_service(repo, MockIdentityProvider::new(PROVIDER_ID), config);

    let minted = svc.mint_nonce().await.expect("mint should succeed");
    assert_eq!(minted.nonce.len(), 43, "32 random bytes base64url-no-pad");
    assert_eq!(minted.expires_in, 90, "expires_in mirrors grants.nonce_ttl");

    // The raw nonce value is never a stored key; only its digest is.
    assert!(
        repo.get_single_use_record(&minted.nonce).await.is_none(),
        "raw nonce must never be stored verbatim"
    );
    let record = repo
        .get_single_use_record(&nonce_key(&minted.nonce))
        .await
        .expect("digest key must be stored");
    assert_eq!(
        record.key,
        nonce_key(&minted.nonce),
        "key is nonce:<sha256hex>"
    );
    assert!(
        record.expires_at > Utc::now(),
        "the stored record must still be live after minting"
    );
}

/// Two mints produce independent values with two independently claimable keys.
#[tokio::test]
async fn mint_nonce_values_are_independent_and_both_claimable() {
    let repo = MockRepository::new();
    let (svc, _audit, repo) =
        make_auditing_service(repo, MockIdentityProvider::new(PROVIDER_ID), make_config());

    let first = svc.mint_nonce().await.expect("first mint");
    let second = svc.mint_nonce().await.expect("second mint");

    assert_ne!(first.nonce, second.nonce, "256-bit values do not collide");
    let first_burned = repo
        .take_single_use(&nonce_key(&first.nonce))
        .await
        .expect("take first");
    let second_burned = repo
        .take_single_use(&nonce_key(&second.nonce))
        .await
        .expect("take second");
    assert!(
        first_burned && second_burned,
        "both records are live and burnable"
    );
}

// ---------------------------------------------------------------------------
// Direct grant: valid once-only flow
// ---------------------------------------------------------------------------

/// Pre-create the binding fixture's user so a successful exchange takes the
/// existing-user path and emits only `TokenExchange` — keeping audit-event
/// sequences free of JIT-registration noise.
async fn precreate_binding_user(repo: &MockRepository) {
    let user = repo
        .create_user(&oidc_exchange_core::domain::NewUser {
            external_id: "binding-subject".to_string(),
            provider: PROVIDER_ID.to_string(),
            email: Some("binding@example.com".to_string()),
            display_name: Some("Binding User".to_string()),
        })
        .await
        .expect("pre-create fixture user");
    assert!(user.id.starts_with("usr_"));
}

/// A valid direct exchange succeeds exactly once. The immediate replay of the
/// identical assertion dies at the nonce control (already burned); presenting
/// the same assertion with a fresh nonce dies at the single-use marker; both
/// rejections are audited naming their control.
#[tokio::test]
async fn direct_grant_succeeds_once_then_rejects_replay() {
    let exp = Utc::now().timestamp() + 600;
    let jti = unique_jti(); // pinned: this test knows the exact jti

    let repo = MockRepository::new();
    let nonce = mint_nonce_over(&repo).await;
    let mut raw = HashMap::new();
    raw.insert("exp".to_string(), json!(exp));
    raw.insert("jti".to_string(), json!(jti.clone()));
    raw.insert("sub".to_string(), json!("binding-subject"));
    raw.insert("nonce".to_string(), json!(nonce.clone()));

    let provider = MockIdentityProvider::new(PROVIDER_ID);
    provider.set_claims(claims_for("RS256", raw)).await;
    let (svc, audit, repo) = make_auditing_service(repo, provider.clone(), make_config());
    precreate_binding_user(&repo).await;

    let first = svc
        .exchange(direct_request())
        .await
        .expect("first use succeeds");
    assert_eq!(
        first.token_type, "Bearer",
        "a valid first use issues tokens"
    );
    assert!(!first.access_token.is_empty());

    // Replay 1: identical assertion — the burned nonce rejects it first.
    let replay = svc
        .exchange(direct_request())
        .await
        .expect_err("identical replay refuses");
    expect_invalid_grant(replay, "missing, expired, or already used");

    // Replay 2: same assertion (same jti), fresh nonce — the marker rejects.
    let fresh_nonce = mint_nonce_over(&repo).await;
    let mut repinned = HashMap::new();
    repinned.insert("exp".to_string(), json!(exp));
    repinned.insert("jti".to_string(), json!(jti.clone()));
    repinned.insert("sub".to_string(), json!("binding-subject"));
    repinned.insert("nonce".to_string(), json!(fresh_nonce.clone()));
    provider.set_claims(claims_for("RS256", repinned)).await;
    let replay2 = svc
        .exchange(direct_request())
        .await
        .expect_err("marker replay refuses");
    expect_invalid_grant(replay2, "already been used");

    // Marker keyed by the provider-namespaced jti digest, expiring at exp.
    let jti_digest = hex::encode(Sha256::digest(jti.as_bytes()));
    let marker = repo
        .get_single_use_record(&format!("assertion:{PROVIDER_ID}:{jti_digest}"))
        .await
        .expect("marker persists after first use");
    assert_eq!(
        marker.expires_at.timestamp(),
        exp,
        "the marker expires at the assertion's own exp"
    );
    // Both nonces are gone: burned on their first (only) use.
    assert!(repo
        .get_single_use_record(&nonce_key(&nonce))
        .await
        .is_none());
    assert!(repo
        .get_single_use_record(&nonce_key(&fresh_nonce))
        .await
        .is_none());

    // One success event, then one ValidationFailed per rejected replay, each
    // naming the control that failed.
    let events = audit.events().await;
    assert_eq!(events.len(), 3, "success plus two rejections");
    assert_eq!(events[0].event_type, AuditEventType::TokenExchange);
    assert_eq!(events[1].event_type, AuditEventType::ValidationFailed);
    assert_eq!(events[1].severity, AuditSeverity::Warning);
    assert!(matches!(&events[1].outcome, AuditOutcome::Failure { .. }));
    assert_eq!(
        events[1].detail.get("check"),
        Some(&json!(assertion::CHECK_NONCE)),
        "the identical replay fails at the nonce control"
    );
    assert_eq!(events[2].event_type, AuditEventType::ValidationFailed);
    assert_eq!(
        events[2].detail.get("check"),
        Some(&json!(assertion::CHECK_SINGLE_USE)),
        "the fresh-nonce replay fails at the marker control"
    );
}

// ---------------------------------------------------------------------------
// Direct grant: nonce negative space
// ---------------------------------------------------------------------------

/// An assertion without any nonce claim rejects at the nonce control, audits
/// that control, and pins no replay marker.
#[tokio::test]
async fn direct_grant_rejects_missing_nonce_claim() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new(PROVIDER_ID);
    provider.set_claims(claims_for("RS256", base_raw())).await; // deliberately nonceless
    let (svc, audit, repo) = make_auditing_service(repo, provider, make_config());

    let err = svc
        .exchange(direct_request())
        .await
        .expect_err("missing nonce rejects");
    expect_invalid_grant(err, "no usable nonce");

    let events = audit.events().await;
    assert_eq!(events.len(), 1, "exactly one validation event");
    assert_eq!(events[0].event_type, AuditEventType::ValidationFailed);
    assert_eq!(
        events[0].detail.get("check"),
        Some(&json!(assertion::CHECK_NONCE))
    );

    let jti_digest = hex::encode(Sha256::digest(b"jti-0"));
    assert!(
        repo.get_single_use_record(&format!("assertion:{PROVIDER_ID}:{jti_digest}"))
            .await
            .is_none(),
        "a rejected binding must not pin a replay marker"
    );
}

/// A nonce this service never issued rejects exactly like a burned one — the
/// absent cases are indistinguishable to the caller.
#[tokio::test]
async fn direct_grant_rejects_unissued_nonce() {
    let repo = MockRepository::new();
    let mut raw = base_raw();
    raw.insert("nonce".to_string(), json!("never-minted-value"));
    let provider = MockIdentityProvider::new(PROVIDER_ID);
    provider.set_claims(claims_for("RS256", raw)).await;
    let (svc, _audit, repo) = make_auditing_service(repo, provider, make_config());

    let err = svc
        .exchange(direct_request())
        .await
        .expect_err("unissued nonce rejects");
    expect_invalid_grant(err, "missing, expired, or already used");

    // Nothing was admitted: no marker exists under this assertion's jti.
    let jti_digest = hex::encode(Sha256::digest(b"jti-0"));
    assert!(
        repo.get_single_use_record(&format!("assertion:{PROVIDER_ID}:{jti_digest}"))
            .await
            .is_none(),
        "no marker may be claimed when the nonce check fails"
    );
}

/// One nonce reused across two different assertions (distinct jti) is caught:
/// only the first spend wins, because take_single_use burns atomically.
#[tokio::test]
async fn direct_grant_rejects_reused_nonce_across_assertions() {
    let repo = MockRepository::new();
    let nonce = mint_nonce_over(&repo).await;

    let mut first_raw = base_raw();
    first_raw.insert("nonce".to_string(), json!(nonce.clone()));
    let first_provider = MockIdentityProvider::new(PROVIDER_ID);
    first_provider
        .set_claims(claims_for("RS256", first_raw))
        .await;
    let (svc, _audit, _repo) = make_auditing_service(repo.clone(), first_provider, make_config());
    svc.exchange(direct_request())
        .await
        .expect("first spend works");

    let mut second_raw = base_raw(); // fresh jti: a different assertion
    second_raw.insert("nonce".to_string(), json!(nonce));
    let second_provider = MockIdentityProvider::new(PROVIDER_ID);
    second_provider
        .set_claims(claims_for("RS256", second_raw))
        .await;
    let (svc, _audit, _repo) = make_auditing_service(repo.clone(), second_provider, make_config());

    let err = svc
        .exchange(direct_request())
        .await
        .expect_err("reused nonce rejects");
    expect_invalid_grant(err, "missing, expired, or already used");
}

// ---------------------------------------------------------------------------
// Shared controls on the direct path
// ---------------------------------------------------------------------------

/// An assertion whose remaining lifetime exceeds the configured ceiling is
/// rejected; one inside the ceiling passes.
#[tokio::test]
async fn lifetime_ceiling_rejects_long_remaining_lifetime() {
    let repo = MockRepository::new();
    let nonce = mint_nonce_over(&repo).await;

    let mut over = base_raw();
    over.insert("exp".to_string(), json!(Utc::now().timestamp() + 7200)); // 2h > 1h
    over.insert("nonce".to_string(), json!(nonce.clone()));
    let provider = MockIdentityProvider::new(PROVIDER_ID);
    provider.set_claims(claims_for("RS256", over)).await;
    let (svc, _audit, repo) = make_auditing_service(repo, provider, config_with("1h"));

    let err = svc
        .exchange(direct_request())
        .await
        .expect_err("over-ceiling refuses");
    expect_invalid_grant(err, "lifetime exceeds the configured maximum");

    // Negative space: fifty minutes to live passes the same one-hour ceiling.
    let nonce = mint_nonce_over(&repo).await;
    let mut under = base_raw();
    under.insert("exp".to_string(), json!(Utc::now().timestamp() + 3000));
    under.insert("nonce".to_string(), json!(nonce));
    let provider = MockIdentityProvider::new(PROVIDER_ID);
    provider.set_claims(claims_for("RS256", under)).await;
    let (svc, _audit, _repo) = make_auditing_service(repo, provider, config_with("1h"));

    let response = svc
        .exchange(direct_request())
        .await
        .expect("under-ceiling accepts");
    assert_eq!(response.token_type, "Bearer");
}

/// Multi-audience rules: `azp` required when `aud` is multi-valued, present
/// `azp` must name this client, and a correct token passes.
#[tokio::test]
async fn azp_rules_enforce_client_binding() {
    let multi_aud = json!(["test-client-id", "sibling-app"]);
    let cases: Vec<(&str, Option<Value>, bool)> = vec![
        ("multi-aud without azp", None, false),
        (
            "multi-aud with sibling azp",
            Some(json!("sibling-app")),
            false,
        ),
        (
            "multi-aud with our own azp",
            Some(json!(MOCK_CLIENT_ID)),
            true,
        ),
    ];

    for (name, azp, should_pass) in cases {
        let repo = MockRepository::new();
        let nonce = mint_nonce_over(&repo).await;
        let mut raw = base_raw();
        raw.insert("aud".to_string(), multi_aud.clone());
        raw.insert("nonce".to_string(), json!(nonce));
        if let Some(value) = azp {
            raw.insert("azp".to_string(), value);
        }
        let provider = MockIdentityProvider::new(PROVIDER_ID);
        provider.set_claims(claims_for("RS256", raw)).await;
        let (svc, _audit, _repo) = make_auditing_service(repo, provider, make_config());

        let result = svc.exchange(direct_request()).await;
        assert_eq!(
            result.is_ok(),
            should_pass,
            "case {name:?} took the wrong branch"
        );
        if let Err(err) = result {
            expect_invalid_grant(err, "azp");
        }
    }

    // Present-but-wrong `azp` rejects even for a single-value audience.
    let repo = MockRepository::new();
    let nonce = mint_nonce_over(&repo).await;
    let mut raw = base_raw();
    raw.insert("aud".to_string(), json!("test-client-id"));
    raw.insert("azp".to_string(), json!("sibling-app"));
    raw.insert("nonce".to_string(), json!(nonce));
    let provider = MockIdentityProvider::new(PROVIDER_ID);
    provider.set_claims(claims_for("RS256", raw)).await;
    let (svc, _audit, _repo) = make_auditing_service(repo, provider, make_config());

    let err = svc
        .exchange(direct_request())
        .await
        .expect_err("foreign azp refuses");
    expect_invalid_grant(err, "does not match this client");
}

/// Compute the RS256 `at_hash` for an access token per OIDC Core §3.1.3.6:
/// base64url-no-pad of the left-most half of SHA-256(ASCII octets).
fn rs256_at_hash(access_token: &str) -> String {
    URL_SAFE_NO_PAD.encode(&Sha256::digest(access_token.as_bytes())[..16])
}

/// `at_hash` verification: correct passes; mismatched rejects with the
/// control audited; no access token skips; EdDSA rejects outright; unknown
/// digest families fail closed.
#[tokio::test]
async fn at_hash_rules_follow_signing_algorithm_and_token_presence() {
    let access_token = "provider-access-token-value";

    // Correct at_hash plus the matching provider access token: accepted.
    let mut raw = base_raw();
    raw.insert("at_hash".to_string(), json!(rs256_at_hash(access_token)));
    let (svc, _audit, _repo, _provider, request) =
        direct_fixture("RS256", raw, Some(access_token)).await;
    let response = svc
        .exchange(request)
        .await
        .expect("correct at_hash accepts");
    assert_eq!(response.token_type, "Bearer");

    // Mismatched at_hash: rejected, with the at_hash control named in audit.
    let mut raw = base_raw();
    raw.insert("at_hash".to_string(), json!(rs256_at_hash("other-token")));
    let (svc, audit, _repo, _provider, request) =
        direct_fixture("RS256", raw, Some(access_token)).await;
    let err = svc.exchange(request).await.expect_err("mismatch refuses");
    expect_invalid_grant(err, "does not match the accompanying access token");
    let events = audit.events().await;
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].detail.get("check"),
        Some(&json!(assertion::CHECK_AT_HASH))
    );

    // The same wrong at_hash with NO access token presented: skipped.
    let mut raw = base_raw();
    raw.insert("at_hash".to_string(), json!(rs256_at_hash("other-token")));
    let (svc, _audit, _repo, _provider, request) = direct_fixture("RS256", raw, None).await;
    svc.exchange(request)
        .await
        .expect("no token means no check");

    // Any at_hash on an EdDSA-signed assertion is refused — with or without
    // an accompanying access token.
    let mut raw = base_raw();
    raw.insert("at_hash".to_string(), json!(rs256_at_hash(access_token)));
    let (svc, _audit, _repo, _provider, request) =
        direct_fixture("EdDSA", raw, Some(access_token)).await;
    let err = svc
        .exchange(request)
        .await
        .expect_err("EdDSA at_hash refuses");
    expect_invalid_grant(err, "unverifiable on an EdDSA-signed assertion");

    let mut raw = base_raw();
    raw.insert("at_hash".to_string(), json!(rs256_at_hash(access_token)));
    let (svc, _audit, _repo, _provider, request) = direct_fixture("EdDSA", raw, None).await;
    let err = svc
        .exchange(request)
        .await
        .expect_err("EdDSA at_hash refuses without a token too");
    expect_invalid_grant(err, "unverifiable on an EdDSA-signed assertion");

    // An algorithm outside every known digest family fails closed.
    let mut raw = base_raw();
    raw.insert("at_hash".to_string(), json!(rs256_at_hash(access_token)));
    let (svc, _audit, _repo, _provider, request) =
        direct_fixture("HS999", raw, Some(access_token)).await;
    let err = svc
        .exchange(request)
        .await
        .expect_err("unknown alg refuses");
    expect_invalid_grant(err, "no verifiable at_hash digest");
}

/// Without a `jti`, the replay marker digests the compact JWT behind the
/// `d:` discriminator, cannot collide with a literal-jti key carrying the
/// same digest, and still blocks a second presentation of the same token.
#[tokio::test]
async fn no_jti_fallback_keys_the_marker_from_the_compact_jwt() {
    // No jti in the raw claims.
    let repo = MockRepository::new();
    let nonce = mint_nonce_over(&repo).await;
    let mut raw = base_raw();
    let removed = raw.remove("jti");
    assert!(removed.is_some(), "base_raw carries a jti to remove");
    raw.insert("nonce".to_string(), json!(nonce));
    let provider = MockIdentityProvider::new(PROVIDER_ID);
    provider.set_claims(claims_for("RS256", raw)).await;
    let (svc, _audit, repo) = make_auditing_service(repo, provider.clone(), make_config());

    let response = svc
        .exchange(direct_request())
        .await
        .expect("exchange succeeds");
    assert_eq!(response.token_type, "Bearer");

    // The presented compact JWT ("header.assertion.signature") is digested.
    let jwt_digest = hex::encode(Sha256::digest(b"header.assertion.signature"));
    let fallback_key = format!("assertion:{PROVIDER_ID}:d:{jwt_digest}");
    let marker = repo
        .get_single_use_record(&fallback_key)
        .await
        .expect("the d:-discriminated fallback marker must exist");
    assert_eq!(marker.key, fallback_key);

    // Negative space: the non-discriminated shape is a different key and is
    // not occupied by this exchange.
    assert!(
        repo.get_single_use_record(&format!("assertion:{PROVIDER_ID}:{jwt_digest}"))
            .await
            .is_none(),
        "the discriminator must separate fallback keys from literal-jti keys"
    );

    // A second presentation of the same token (fresh nonce) is still a
    // replay under the fallback key.
    let fresh_nonce = mint_nonce_over(&repo).await;
    let mut repinned = base_raw();
    repinned.remove("jti");
    repinned.insert("nonce".to_string(), json!(fresh_nonce));
    provider.set_claims(claims_for("RS256", repinned)).await;
    let replay = svc
        .exchange(direct_request())
        .await
        .expect_err("fallback replay refuses");
    expect_invalid_grant(replay, "already been used");
}

// ---------------------------------------------------------------------------
// Code path: shared controls, no nonce requirement
// ---------------------------------------------------------------------------

/// The authorization-code path runs the shared controls but requires no
/// nonce: it succeeds with nonceless claims, replays of the same assertion
/// reject as single_use, and unused nonces are left untouched by the code
/// path entirely.
#[tokio::test]
async fn code_path_binds_without_nonce_and_detects_replays() {
    let repo = MockRepository::new();
    let untouched_nonce = mint_nonce_over(&repo).await;

    // Pinned claims carry no nonce at all: fine for the code path.
    let provider = MockIdentityProvider::new(PROVIDER_ID);
    provider.set_claims(claims_for("RS256", base_raw())).await;
    let (svc, audit, repo) = make_auditing_service(repo, provider, make_config());
    precreate_binding_user(&repo).await;

    let first = svc
        .exchange(code_request())
        .await
        .expect("code exchange succeeds");
    assert_eq!(first.token_type, "Bearer");

    // Same pinned claims returned again → same jti → replay.
    let replay = svc
        .exchange(code_request())
        .await
        .expect_err("code replay refuses");
    expect_invalid_grant(replay, "already been used");

    // The code path never consumed the unrelated live nonce.
    assert!(
        repo.get_single_use_record(&nonce_key(&untouched_nonce))
            .await
            .is_some_and(|record| record.expires_at > Utc::now()),
        "the code path must not burn nonces"
    );

    // Success emitted TokenExchange; the replay emitted ValidationFailed
    // naming the single-use control.
    let events = audit.events().await;
    assert_eq!(events.len(), 2, "success event plus replay rejection");
    assert_eq!(events[0].event_type, AuditEventType::TokenExchange);
    assert_eq!(events[1].event_type, AuditEventType::ValidationFailed);
    assert_eq!(
        events[1].detail.get("check"),
        Some(&json!(assertion::CHECK_SINGLE_USE))
    );
}

// ---------------------------------------------------------------------------
// Store failures propagate as typed errors, distinct from rejections
// ---------------------------------------------------------------------------

/// Session-repository decorator that can fail each single-use operation on
/// demand, modelling an unreachable store mid-binding.
struct SingleUseFailRepo {
    inner: MockRepository,
    fail_take: AtomicBool,
    fail_put: AtomicBool,
}

impl SingleUseFailRepo {
    fn armed(inner: MockRepository, fail_take: bool, fail_put: bool) -> Self {
        Self {
            inner,
            fail_take: AtomicBool::new(fail_take),
            fail_put: AtomicBool::new(fail_put),
        }
    }
}

#[async_trait]
impl SessionRepository for SingleUseFailRepo {
    async fn store_refresh_token(&self, session: &Session) -> Result<()> {
        self.inner.store_refresh_token(session).await
    }

    async fn get_session_by_refresh_token(&self, token_hash: &str) -> Result<Option<Session>> {
        self.inner.get_session_by_refresh_token(token_hash).await
    }

    async fn revoke_session(&self, token_hash: &str) -> Result<()> {
        self.inner.revoke_session(token_hash).await
    }

    async fn revoke_all_user_sessions(&self, user_id: &str) -> Result<()> {
        self.inner.revoke_all_user_sessions(user_id).await
    }

    async fn count_active_sessions(&self) -> Result<u64> {
        self.inner.count_active_sessions().await
    }

    async fn cleanup_expired_sessions(&self) -> Result<u64> {
        self.inner.cleanup_expired_sessions().await
    }

    async fn put_single_use(&self, key: &str, expires_at: chrono::DateTime<Utc>) -> Result<bool> {
        if self.fail_put.load(Ordering::SeqCst) {
            return Err(Error::StoreError {
                detail: "simulated put failure".to_string(),
            });
        }
        self.inner.put_single_use(key, expires_at).await
    }

    async fn take_single_use(&self, key: &str) -> Result<bool> {
        if self.fail_take.load(Ordering::SeqCst) {
            return Err(Error::StoreError {
                detail: "simulated take failure".to_string(),
            });
        }
        self.inner.take_single_use(key).await
    }
}

/// Build a service whose session repository is the failing decorator over
/// `repo` with the given arms armed, and the pinned `provider` registered.
/// Hands back the audit log so tests can assert that infrastructure failures
/// never audit as binding rejections.
fn service_with_failing_store(
    repo: MockRepository,
    provider: MockIdentityProvider,
    fail_take: bool,
    fail_put: bool,
) -> (AppService, MockAuditLog) {
    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    providers.insert(PROVIDER_ID.to_string(), Box::new(provider));
    let audit = MockAuditLog::new();
    let svc = AppService::new(
        Box::new(repo.clone()),
        Box::new(SingleUseFailRepo::armed(repo, fail_take, fail_put)),
        Box::new(MockKeyManager::new()),
        Box::new(audit.clone()),
        Box::new(MockUserSync::new()),
        providers,
        make_config(),
    );
    (svc, audit)
}

/// A store failure during nonce consumption surfaces as `StoreError`, never
/// disguised as an `InvalidGrant`, leaves no rejection audit trail, and does
/// not burn the nonce.
#[tokio::test]
async fn store_failure_on_nonce_take_propagates_typed_error() {
    // Mint the nonce for real so the only failure is the armed take.
    let repo = MockRepository::new();
    let nonce = mint_nonce_over(&repo).await;
    let mut raw = base_raw();
    raw.insert("nonce".to_string(), json!(nonce.clone()));

    let provider = MockIdentityProvider::new(PROVIDER_ID);
    provider.set_claims(claims_for("RS256", raw)).await;
    let (svc, audit) = service_with_failing_store(repo.clone(), provider, true, false);

    let err = svc
        .exchange(direct_request())
        .await
        .expect_err("store failure surfaces");
    match err {
        Error::StoreError { detail } => {
            assert!(
                detail.contains("simulated"),
                "typed store error carries detail"
            )
        }
        other => panic!("expected StoreError, got: {other:?}"),
    }

    // No ValidationFailed rejection was audited: infrastructure ≠ client fault.
    let events = audit.events().await;
    assert!(
        events
            .iter()
            .all(|e| e.event_type != AuditEventType::ValidationFailed),
        "store failures must not be audited as binding rejections"
    );
    // The nonce survives the failed attempt.
    assert!(
        repo.get_single_use_record(&nonce_key(&nonce))
            .await
            .is_some(),
        "a failed take must not consume the nonce"
    );
}

/// A store failure while claiming the marker propagates typed as well — and
/// the nonce, already burned by the earlier step, stays consumed (a partial
/// run never admits a replay; it just costs the client one round trip).
#[tokio::test]
async fn store_failure_on_marker_put_propagates_and_keeps_nonce_burned() {
    let repo = MockRepository::new();
    let nonce = mint_nonce_over(&repo).await;

    let mut raw = base_raw();
    raw.insert("nonce".to_string(), json!(nonce.clone()));
    let provider = MockIdentityProvider::new(PROVIDER_ID);
    provider.set_claims(claims_for("RS256", raw)).await;
    let (svc, _audit) = service_with_failing_store(repo.clone(), provider, false, true);

    let err = svc
        .exchange(direct_request())
        .await
        .expect_err("put failure surfaces");
    match err {
        Error::StoreError { detail } => {
            assert!(
                detail.contains("simulated"),
                "typed detail carries the failure"
            )
        }
        other => panic!("expected StoreError from the marker claim, got: {other:?}"),
    }

    // Order proof: the nonce step ran before the failing marker claim.
    assert!(
        repo.get_single_use_record(&nonce_key(&nonce))
            .await
            .is_none(),
        "the nonce was already consumed before the marker claim failed"
    );
}
