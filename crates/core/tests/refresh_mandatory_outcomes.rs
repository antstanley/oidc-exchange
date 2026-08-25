//! S3 companion to `exchange_mandatory_outcomes.rs`: the refresh flow's
//! security outcomes — success, suspension (on both the rotation and the
//! rotation-disabled gates), and reuse — ride the mandatory audit channel, so a
//! raised `emit_threshold` can no longer drop them and a sink failure follows
//! the `audit.durability` contract. The Debug-level `ValidationFailed` refusals
//! deliberately stay on the best-effort channel.

use std::collections::HashMap;

use sha2::{Digest, Sha256};

use oidc_exchange_core::config::{Config, RawConfig};
use oidc_exchange_core::domain::{
    AuditEventType, AuditOutcome, NewUser, Session, UserPatch, UserStatus,
};
use oidc_exchange_core::error::Error;
use oidc_exchange_core::ports::{IdentityProvider, SessionRepository, UserRepository};
use oidc_exchange_core::service::refresh::RefreshRequest;
use oidc_exchange_core::service::AppService;
use oidc_exchange_test_utils::{
    MockAuditLog, MockIdentityProvider, MockKeyManager, MockRateLimiter, MockRepository,
    MockUserSync,
};

/// Base config with a resolvable issuer, then per-test knobs applied on the raw
/// tree before resolution: `emit_threshold` (raised to prove the mandatory
/// channel bypasses it), `durability`, and `refresh_rotation`.
fn base_raw() -> RawConfig {
    let mut raw: RawConfig = toml::from_str(include_str!("../../../config/default.toml"))
        .expect("default config deserializes");
    raw.server.issuer = "https://auth.test".to_string();
    raw
}

/// `emit_threshold = "error"` sits *above* (more severe than) the security
/// outcomes' severities — reuse/suspension are `Warning` (4), success is `Info`
/// (6), and `error` is 3 — so the old best-effort channel would have dropped
/// every one of them. The mandatory channel ignores the threshold.
fn raised_threshold_config() -> Config {
    let mut raw = base_raw();
    raw.audit.emit_threshold = "error".to_string();
    Config::resolve(raw).expect("test config resolves")
}

fn service(repo: MockRepository, audit: MockAuditLog, config: Config) -> AppService {
    let provider = MockIdentityProvider::new("mock");
    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    providers.insert(provider.provider_id().to_owned(), Box::new(provider));
    AppService::new(
        Box::new(repo.clone()),
        Box::new(repo),
        Box::new(MockKeyManager::new()),
        Box::new(audit),
        Box::new(MockUserSync::new()),
        Box::new(MockRateLimiter::new()),
        providers,
        config,
    )
}

fn hash_of(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// Seed a `mock`-provider user plus one live refresh-token session redeemable
/// with `raw_token`, directly through the repository (no exchange, so no
/// exchange-plane audit events pollute the log).
async fn seed_live_session(repo: &MockRepository, raw_token: &str) -> String {
    let user = repo
        .create_user(&NewUser {
            external_id: "reuse-subject".into(),
            provider: "mock".into(),
            email: Some("subject@example.com".into()),
            display_name: None,
        })
        .await
        .expect("seed user");
    let session = Session {
        user_id: user.id.clone(),
        refresh_token_hash: oidc_exchange_core::Secret::new(hash_of(raw_token)),
        family_id: oidc_exchange_core::domain::new_family_id(),
        generation: 0,
        provider: "mock".into(),
        expires_at: chrono::Utc::now() + chrono::Duration::days(1),
        rotated_at: None,
        device_id: None,
        user_agent: None,
        ip_address: None,
        created_at: chrono::Utc::now(),
    };
    repo.store_refresh_token(&session)
        .await
        .expect("seed live session");
    user.id
}

async fn suspend(repo: &MockRepository, user_id: &str) {
    repo.update_user(
        user_id,
        &UserPatch {
            email: None,
            display_name: None,
            metadata: None,
            claims: None,
            status: Some(UserStatus::Suspended),
        },
    )
    .await
    .expect("suspend user");
}

fn refresh_request(raw_token: &str) -> RefreshRequest {
    RefreshRequest {
        refresh_token: raw_token.to_string(),
        client_addr: oidc_exchange_core::domain::ClientAddr::Peer("203.0.113.7".parse().unwrap()),
        user_agent: Some("agent/1.0".to_string()),
        device_id: None,
    }
}

fn count_of(events: &[oidc_exchange_core::domain::AuditEvent], kind: AuditEventType) -> usize {
    events.iter().filter(|e| e.event_type == kind).count()
}

/// Refresh success rides the mandatory channel: with `emit_threshold` raised to
/// `error`, the `TokenRefresh` (Info) event is still emitted.
#[tokio::test]
async fn refresh_success_survives_raised_emit_threshold() {
    let repo = MockRepository::new();
    let audit = MockAuditLog::new();
    let audit_view = audit.clone();
    let svc = service(repo.clone(), audit, raised_threshold_config());
    let user_id = seed_live_session(&repo, "live-token").await;
    assert!(!user_id.is_empty());

    svc.refresh(refresh_request("live-token"))
        .await
        .expect("refresh should succeed");

    let events = audit_view.events().await;
    assert_eq!(
        count_of(&events, AuditEventType::TokenRefresh),
        1,
        "TokenRefresh must ride the mandatory channel and survive emit_threshold = error: {events:#?}"
    );
}

/// Suspension on the rotation path rides the mandatory channel.
#[tokio::test]
async fn refresh_suspension_rotation_path_survives_raised_emit_threshold() {
    let repo = MockRepository::new();
    let audit = MockAuditLog::new();
    let audit_view = audit.clone();
    // Default config has refresh_rotation = true.
    let svc = service(repo.clone(), audit, raised_threshold_config());
    let user_id = seed_live_session(&repo, "live-token").await;
    suspend(&repo, &user_id).await;

    let err = svc
        .refresh(refresh_request("live-token"))
        .await
        .expect_err("a suspended user's refresh must fail");
    assert!(matches!(err, Error::UserSuspended { .. }), "got {err:?}");

    let events = audit_view.events().await;
    assert_eq!(
        count_of(&events, AuditEventType::UserSuspended),
        1,
        "UserSuspended (rotation path) must ride the mandatory channel: {events:#?}"
    );
    assert!(matches!(
        events
            .iter()
            .find(|e| e.event_type == AuditEventType::UserSuspended)
            .map(|e| &e.outcome),
        Some(AuditOutcome::Failure(_))
    ));
}

/// Suspension on the rotation-disabled path (`token.refresh_rotation = false`,
/// live only because task 01 made the switch functional) rides the mandatory
/// channel too.
#[tokio::test]
async fn refresh_suspension_rotation_disabled_path_survives_raised_emit_threshold() {
    let repo = MockRepository::new();
    let audit = MockAuditLog::new();
    let audit_view = audit.clone();
    let config = {
        let mut raw = base_raw();
        raw.audit.emit_threshold = "error".to_string();
        raw.token.refresh_rotation = false;
        let config = Config::resolve(raw).expect("test config resolves");
        assert!(
            !config.token.refresh_rotation,
            "refresh_rotation = false must survive resolution (task 01) for this path to run"
        );
        config
    };
    let svc = service(repo.clone(), audit, config);
    let user_id = seed_live_session(&repo, "live-token").await;
    suspend(&repo, &user_id).await;

    let err = svc
        .refresh(refresh_request("live-token"))
        .await
        .expect_err("a suspended user's refresh must fail");
    assert!(matches!(err, Error::UserSuspended { .. }), "got {err:?}");

    let events = audit_view.events().await;
    assert_eq!(
        count_of(&events, AuditEventType::UserSuspended),
        1,
        "UserSuspended (rotation-disabled path) must ride the mandatory channel: {events:#?}"
    );
}

/// A rotation-disabled refresh of an active user still succeeds without minting
/// a replacement — proof the rotation-disabled path is the one running above.
#[tokio::test]
async fn refresh_rotation_disabled_success_mints_no_replacement() {
    let repo = MockRepository::new();
    let audit = MockAuditLog::new();
    let config = {
        let mut raw = base_raw();
        raw.token.refresh_rotation = false;
        Config::resolve(raw).expect("test config resolves")
    };
    let svc = service(repo.clone(), audit, config);
    seed_live_session(&repo, "live-token").await;

    let response = svc
        .refresh(refresh_request("live-token"))
        .await
        .expect("rotation-disabled refresh should succeed");
    assert!(
        response.refresh_token.is_none(),
        "the rotation-disabled path must not mint a replacement refresh token"
    );
}

/// Reuse rides the mandatory channel: with `emit_threshold` raised to `error`,
/// the `RefreshTokenReuse` (Warning) event is still emitted, and the family is
/// revoked before the emission.
#[tokio::test]
async fn refresh_reuse_survives_raised_emit_threshold() {
    let repo = MockRepository::new();
    let audit = MockAuditLog::new();
    let audit_view = audit.clone();
    let svc = service(repo.clone(), audit, raised_threshold_config());

    // Seed a live session, rotate it once (retiring gen 0), then backdate gen
    // 0's retirement past the grace window so re-presenting it is unambiguous
    // reuse.
    seed_live_session(&repo, "gen0-token").await;
    svc.refresh(refresh_request("gen0-token"))
        .await
        .expect("first rotation");
    assert!(
        repo.backdate_retirement(&hash_of("gen0-token"), 11).await,
        "gen 0's retirement record must exist to be backdated"
    );

    let err = svc
        .refresh(refresh_request("gen0-token"))
        .await
        .expect_err("out-of-grace re-presentation is reuse");
    assert!(matches!(err, Error::InvalidToken { .. }), "got {err:?}");

    assert!(
        repo.get_all_sessions().await.is_empty(),
        "reuse must revoke the family's live generation"
    );
    let events = audit_view.events().await;
    assert_eq!(
        count_of(&events, AuditEventType::RefreshTokenReuse),
        1,
        "RefreshTokenReuse must ride the mandatory channel and survive emit_threshold = error: {events:#?}"
    );
}

/// Negative space: the shared `ValidationFailed` refusal path stays on the
/// best-effort channel at Debug, so an unknown-token refusal is filtered out by
/// the default `emit_threshold` (Debug 7 > Info 6) — no event is recorded.
#[tokio::test]
async fn validation_failed_refusal_stays_best_effort_and_is_filtered() {
    let repo = MockRepository::new();
    let audit = MockAuditLog::new();
    let audit_view = audit.clone();
    // Default emit_threshold (info); nothing seeded, so the token is unknown.
    let svc = service(repo, audit, Config::resolve(base_raw()).expect("resolves"));

    let err = svc
        .refresh(refresh_request("never-issued-token"))
        .await
        .expect_err("an unknown token is refused");
    assert!(matches!(err, Error::InvalidToken { .. }), "got {err:?}");

    let events = audit_view.events().await;
    assert_eq!(
        count_of(&events, AuditEventType::ValidationFailed),
        0,
        "the Debug ValidationFailed refusal must stay best-effort and be filtered by emit_threshold: {events:#?}"
    );
}

/// Durability: reuse revokes the family *before* the emission, so even when a
/// durability-enforced emission fails, the family is already gone and the error
/// propagates.
#[tokio::test]
async fn reuse_revokes_family_even_when_enforce_durability_emission_fails() {
    let repo = MockRepository::new();
    let audit = MockAuditLog::new();
    let config = {
        let mut raw = base_raw();
        raw.audit.durability = "enforce".to_string();
        Config::resolve(raw).expect("test config resolves")
    };
    let svc = service(repo.clone(), audit.clone(), config);

    // Seed + rotate + backdate with the sink healthy, so the setup lands.
    seed_live_session(&repo, "gen0-token").await;
    svc.refresh(refresh_request("gen0-token"))
        .await
        .expect("first rotation");
    assert!(repo.backdate_retirement(&hash_of("gen0-token"), 11).await);

    // Now fail the sink and present the retired token: revoke-before-emit means
    // the family dies before the (now failing) mandatory emission propagates.
    audit.set_fail_mode(true).await;
    let err = svc
        .refresh(refresh_request("gen0-token"))
        .await
        .expect_err("enforce durability turns the failed mandatory emission into an error");
    assert!(
        matches!(err, Error::SecurityAuditDurability { .. }),
        "got {err:?}"
    );
    assert!(
        repo.get_all_sessions().await.is_empty(),
        "the reused family must be revoked before the emission failure propagates"
    );
}

/// Durability: a mandatory success emission that fails under `enforce` fails the
/// request, while under `observe` it records degradation and the flow's own
/// outcome stands.
#[tokio::test]
async fn refresh_success_follows_durability_contract_on_sink_failure() {
    for (durability, expect_err) in [("enforce", true), ("observe", false)] {
        let repo = MockRepository::new();
        let audit = MockAuditLog::new();
        let config = {
            let mut raw = base_raw();
            raw.audit.durability = durability.to_string();
            Config::resolve(raw).expect("test config resolves")
        };
        let svc = service(repo.clone(), audit.clone(), config);
        seed_live_session(&repo, "live-token").await;

        audit.set_fail_mode(true).await;
        let result = svc.refresh(refresh_request("live-token")).await;
        assert_eq!(
            result.is_err(),
            expect_err,
            "durability = {durability}: success emission failure handling"
        );
        if durability == "observe" {
            let response = result.expect("observe: the refresh outcome stands");
            assert!(
                response.refresh_token.is_some(),
                "observe: rotation still returns a replacement token"
            );
        }
    }
}
