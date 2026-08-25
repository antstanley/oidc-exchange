use std::collections::HashMap;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{Duration, Utc};
use sha2::{Digest, Sha256};

use oidc_exchange_core::config::{
    Config, RawAuditConfig, RawConfig, RawRegistrationConfig, RawServerConfig, RawTelemetryConfig,
    RawTokenConfig,
};
use oidc_exchange_core::domain::{
    is_valid_family_id, AccessTokenClaims, AuditEventType, AuditOutcome, AuditSeverity,
    RefreshResolution, Session, UserPatch, UserStatus,
};
use oidc_exchange_core::error::{Error, Result};
use oidc_exchange_core::ports::{IdentityProvider, SessionRepository, UserRepository};
use oidc_exchange_core::service::exchange::{ExchangeCredential, ExchangeRequest};
use oidc_exchange_core::service::refresh::RefreshRequest;
use oidc_exchange_core::service::AppService;

use oidc_exchange_test_utils::{
    MockAuditLog, MockIdentityProvider, MockKeyManager, MockRateLimiter, MockRepository,
    MockUserSync,
};

fn base_raw_config() -> RawConfig {
    RawConfig {
        server: RawServerConfig {
            host: "127.0.0.1".to_string(),
            port: 8080,
            issuer: "https://auth.test.com".to_string(),
            role: "all".to_string(),
            request_timeout: "30s".to_string(),
            base_path: None,
            ..RawServerConfig::default()
        },
        registration: RawRegistrationConfig {
            mode: "open".to_string(),
            domain_allowlist: None,
        },
        token: RawTokenConfig {
            access_token_ttl: "15m".to_string(),
            refresh_token_ttl: "30d".to_string(),
            audience: "https://api.test.com".to_string(),
            custom_claims: None,
            ..RawTokenConfig::default()
        },
        audit: RawAuditConfig {
            adapter: "noop".to_string(),
            blocking_threshold: "warning".to_string(),
            emit_threshold: "info".to_string(),
            sqs: None,
            ..RawAuditConfig::default()
        },
        telemetry: RawTelemetryConfig {
            enabled: false,
            exporter: "none".to_string(),
            endpoint: None,
            service_name: None,
            sample_rate: None,
            protocol: None,
        },
        ..RawConfig::default()
    }
}

fn make_config() -> Config {
    Config::resolve(base_raw_config()).expect("test config should resolve")
}
fn make_service(repo: MockRepository, provider: MockIdentityProvider) -> AppService {
    let provider_id = provider.provider_id().to_string();
    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    providers.insert(provider_id, Box::new(provider));

    AppService::new(
        Box::new(repo.clone()),
        Box::new(repo),
        Box::new(MockKeyManager::new()),
        Box::new(MockAuditLog::new()),
        Box::new(MockUserSync::new()),
        Box::new(MockRateLimiter::new()),
        providers,
        make_config(),
    )
}

/// Builds a service whose `AuditLog` is a caller-supplied `MockAuditLog`, so
/// the test can inspect recorded events after the refresh runs.
fn make_service_with_audit(
    repo: MockRepository,
    provider: MockIdentityProvider,
    config: Config,
    audit: MockAuditLog,
) -> AppService {
    let provider_id = provider.provider_id().to_string();
    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    providers.insert(provider_id, Box::new(provider));

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

/// Helper: perform an exchange to get a refresh token, then return it along
/// with the service and repo for further testing.
async fn exchange_and_get_refresh_token(_repo: &MockRepository, svc: &AppService) -> String {
    let request = ExchangeRequest {
        provider_access_token: None,
        credential: ExchangeCredential::AuthorizationCode {
            code: "auth-code-123".to_string(),
            redirect_uri: "https://app.test.com/callback".to_string(),
        },
        provider: "mock".to_string(),
        client_addr: oidc_exchange_core::domain::ClientAddr::Unknown,
        user_agent: None,
        device_id: None,
    };
    let response = svc
        .exchange(request)
        .await
        .expect("exchange should succeed");
    response
        .refresh_token
        .expect("should have a refresh token")
        .into_inner()
}

#[tokio::test]
async fn refresh_happy_path_returns_new_access_token() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let svc = make_service(repo.clone(), provider);

    // First do an exchange to get a refresh token
    let refresh_token = exchange_and_get_refresh_token(&repo, &svc).await;

    // Now use the refresh token
    let request = RefreshRequest {
        refresh_token: refresh_token.clone(),
        ..Default::default()
    };
    let response = svc.refresh(request).await.expect("refresh should succeed");

    // Verify the response
    assert_eq!(response.token_type, "Bearer");
    assert_eq!(response.expires_in, 900); // 15m = 900s
    let replacement = response
        .refresh_token
        .expect("rotation must return a replacement refresh token")
        .into_inner();
    assert!(
        replacement != refresh_token,
        "the replacement must be a fresh opaque token, not the presented one"
    );
    assert!(!response.access_token.is_empty());

    // Access token should be a valid JWT structure (3 dot-separated parts)
    let parts: Vec<&str> = response.access_token.split('.').collect();
    assert_eq!(parts.len(), 3, "JWT should have 3 parts");

    // Decode and verify the header: refresh mints the same RFC 9068
    // access-token media type exchange does.
    let header_bytes = URL_SAFE_NO_PAD
        .decode(parts[0])
        .expect("header should be valid base64url");
    let header: serde_json::Value =
        serde_json::from_slice(&header_bytes).expect("header should deserialize");
    assert_eq!(
        header["typ"], "at+jwt",
        "refreshed tokens must also be at+jwt"
    );

    // Decode and verify the payload claims
    let claims = decode_claims(&response.access_token);
    assert_eq!(claims.iss, "https://auth.test.com");
    assert_eq!(claims.aud, "https://api.test.com");
    assert!(claims.sub.starts_with("usr_"));

    // The sub should match the user created during exchange
    let users = repo.get_all_users().await;
    assert_eq!(users.len(), 1);
    assert_eq!(claims.sub, users[0].id);

    // The presented generation no longer resolves as live; the replacement does.
    let presented_hash = hex::encode(Sha256::digest(refresh_token.as_bytes()));
    let replacement_hash = hex::encode(Sha256::digest(replacement.as_bytes()));
    let resolution = repo
        .resolve_refresh_token(&presented_hash)
        .await
        .expect("classify presented hash");
    assert!(
        matches!(resolution, RefreshResolution::Superseded { .. }),
        "immediately after rotation the old generation is superseded (grace-eligible), got {resolution:?}"
    );
    let resolution = repo
        .resolve_refresh_token(&replacement_hash)
        .await
        .expect("classify replacement hash");
    assert!(
        matches!(resolution, RefreshResolution::Live(_)),
        "the replacement must be the family's live generation"
    );

    // The refreshed token must keep the session binding stable: its `sid`
    // is the family id, which rotation never moves, so revocation by access
    // token keeps naming this credential chain.
    let stored = repo
        .get_session_by_refresh_token(&oidc_exchange_core::secret::Secret::new(
            replacement_hash.clone(),
        ))
        .await
        .expect("lookup should not error")
        .expect("the replacement generation must be live after a refresh");
    assert_eq!(
        claims.sid, stored.family_id,
        "a refreshed access token must carry the family's stable identifier"
    );
}

#[tokio::test]
async fn refresh_expired_token_returns_invalid_token() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let svc = make_service(repo.clone(), provider);

    // Do an exchange to create a user and session
    let refresh_token = exchange_and_get_refresh_token(&repo, &svc).await;

    // Manually expire the session by replacing it with an expired one
    let token_hash = hex::encode(Sha256::digest(refresh_token.as_bytes()));
    let sessions = repo.get_all_sessions().await;
    let original_session = sessions
        .iter()
        .find(|s| s.refresh_token_hash.expose() == &token_hash)
        .expect("session should exist");

    // Revoke the original and store an expired copy
    repo.revoke_session(&oidc_exchange_core::Secret::new(token_hash.clone()))
        .await
        .expect("revoke should succeed");

    let expired_session = Session {
        expires_at: Utc::now() - Duration::hours(1),
        ..original_session.clone()
    };
    repo.store_refresh_token(&expired_session)
        .await
        .expect("store should succeed");

    // Now try to refresh with the expired token
    let request = RefreshRequest {
        refresh_token,
        ..Default::default()
    };
    let err = svc
        .refresh(request)
        .await
        .expect_err("refresh should fail for expired token");

    match err {
        Error::InvalidToken { .. } => {} // expected
        other => panic!("expected InvalidToken, got: {:?}", other),
    }
}

#[tokio::test]
async fn refresh_unknown_token_returns_invalid_token() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let svc = make_service(repo, provider);

    // Try to refresh with a token that was never stored
    let request = RefreshRequest {
        refresh_token: "this-token-does-not-exist".to_string(),
        ..Default::default()
    };
    let err = svc
        .refresh(request)
        .await
        .expect_err("refresh should fail for unknown token");

    match err {
        Error::InvalidToken { .. } => {} // expected
        other => panic!("expected InvalidToken, got: {:?}", other),
    }
}

#[tokio::test]
async fn refresh_suspended_user_returns_user_suspended() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let svc = make_service(repo.clone(), provider);

    // Exchange to create user and session
    let refresh_token = exchange_and_get_refresh_token(&repo, &svc).await;

    // Suspend the user
    let users = repo.get_all_users().await;
    let user_id = users[0].id.clone();
    repo.update_user(
        &user_id,
        &UserPatch {
            status: Some(UserStatus::Suspended),
            email: None,
            display_name: None,
            metadata: None,
            claims: None,
        },
    )
    .await
    .expect("update should succeed");

    // Now try to refresh
    let request = RefreshRequest {
        refresh_token,
        ..Default::default()
    };
    let err = svc
        .refresh(request)
        .await
        .expect_err("refresh should fail for suspended user");

    match err {
        Error::UserSuspended { user_id: id } => {
            assert_eq!(id, user_id);
        }
        other => panic!("expected UserSuspended, got: {:?}", other),
    }
}

/// Negative space: an unknown-token refresh under the default `info`
/// `emit_threshold` audits nothing — `ValidationFailed` is emitted at
/// `debug`, which the default threshold drops before any adapter sees it.
#[tokio::test]
async fn refresh_unknown_token_under_default_threshold_emits_nothing() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let audit = MockAuditLog::new();
    let audit_clone = audit.clone();
    let svc = make_service_with_audit(repo, provider, make_config(), audit);

    let request = RefreshRequest {
        refresh_token: "this-token-does-not-exist".to_string(),
        client_addr: oidc_exchange_core::domain::ClientAddr::Peer("203.0.113.20".parse().unwrap()),
        user_agent: Some("test-agent/1.0".to_string()),
        device_id: None,
    };
    svc.refresh(request)
        .await
        .expect_err("refresh should fail for unknown token");

    let events = audit_clone.events().await;
    assert!(
        events.is_empty(),
        "default info threshold must suppress debug-severity ValidationFailed, got: {:?}",
        events.iter().map(|e| &e.event_type).collect::<Vec<_>>()
    );
}

/// Lowering `[audit] emit_threshold` to `debug` surfaces the
/// `ValidationFailed` event that the default threshold suppresses, carrying
/// the request's ip/ua and a `Failure` outcome.
#[tokio::test]
async fn refresh_unknown_token_under_debug_threshold_emits_validation_failed() {
    let config = Config::resolve(RawConfig {
        audit: oidc_exchange_core::config::RawAuditConfig {
            adapter: "noop".to_string(),
            blocking_threshold: "warning".to_string(),
            emit_threshold: "debug".to_string(),
            sqs: None,
            ..RawAuditConfig::default()
        },
        ..base_raw_config()
    })
    .expect("test config should resolve");

    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let audit = MockAuditLog::new();
    let audit_clone = audit.clone();
    let svc = make_service_with_audit(repo, provider, config, audit);

    let request = RefreshRequest {
        refresh_token: "this-token-does-not-exist".to_string(),
        client_addr: oidc_exchange_core::domain::ClientAddr::Peer("203.0.113.21".parse().unwrap()),
        user_agent: Some("test-agent/2.0".to_string()),
        device_id: None,
    };
    svc.refresh(request)
        .await
        .expect_err("refresh should fail for unknown token");

    let events = audit_clone.events().await;
    assert_eq!(
        events.len(),
        1,
        "lowering the threshold to debug must surface exactly one event, got: {:?}",
        events.iter().map(|e| &e.event_type).collect::<Vec<_>>()
    );
    assert_eq!(events[0].event_type, AuditEventType::ValidationFailed);
    match &events[0].outcome {
        AuditOutcome::Failure(_) => {}
        other => panic!("expected Failure outcome, got: {:?}", other),
    }
    assert_eq!(events[0].ip_address.as_deref(), Some("203.0.113.21"));
    assert_eq!(events[0].user_agent.as_deref(), Some("test-agent/2.0"));
}

/// An expired-token refresh under a lowered `emit_threshold` also audits
/// `ValidationFailed`, this time with the session's user id as actor since
/// the session (and therefore the user id) was already resolved.
#[tokio::test]
async fn refresh_expired_token_under_debug_threshold_emits_validation_failed_with_actor() {
    let config = Config::resolve(RawConfig {
        audit: oidc_exchange_core::config::RawAuditConfig {
            adapter: "noop".to_string(),
            blocking_threshold: "warning".to_string(),
            emit_threshold: "debug".to_string(),
            sqs: None,
            ..RawAuditConfig::default()
        },
        ..base_raw_config()
    })
    .expect("test config should resolve");

    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let svc = make_service(repo.clone(), provider);

    let refresh_token = exchange_and_get_refresh_token(&repo, &svc).await;

    let token_hash = hex::encode(Sha256::digest(refresh_token.as_bytes()));
    let sessions = repo.get_all_sessions().await;
    let original_session = sessions
        .iter()
        .find(|s| s.refresh_token_hash.expose() == &token_hash)
        .expect("session should exist");
    let user_id = original_session.user_id.clone();

    repo.revoke_session(&oidc_exchange_core::Secret::new(token_hash.clone()))
        .await
        .expect("revoke should succeed");
    let expired_session = Session {
        expires_at: Utc::now() - Duration::hours(1),
        ..original_session.clone()
    };
    repo.store_refresh_token(&expired_session)
        .await
        .expect("store should succeed");

    let provider2 = MockIdentityProvider::new("mock");
    let audit = MockAuditLog::new();
    let audit_clone = audit.clone();
    let svc2 = make_service_with_audit(repo, provider2, config, audit);

    let request = RefreshRequest {
        refresh_token,
        ..Default::default()
    };
    svc2.refresh(request)
        .await
        .expect_err("refresh should fail for expired token");

    let events = audit_clone.events().await;
    assert_eq!(events.len(), 1, "expected exactly one audit event");
    assert_eq!(events[0].event_type, AuditEventType::ValidationFailed);
    assert_eq!(events[0].actor.as_deref(), Some(user_id.as_str()));
}

/// A suspended-user refresh emits `UserSuspended` (default threshold covers
/// it, since it is not gated behind `emit_threshold`), carrying the user id
/// as actor and the request's ip/ua.
#[tokio::test]
async fn refresh_suspended_user_emits_user_suspended_event() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let svc = make_service(repo.clone(), provider);

    let refresh_token = exchange_and_get_refresh_token(&repo, &svc).await;

    let users = repo.get_all_users().await;
    let user_id = users[0].id.clone();
    repo.update_user(
        &user_id,
        &UserPatch {
            status: Some(UserStatus::Suspended),
            email: None,
            display_name: None,
            metadata: None,
            claims: None,
        },
    )
    .await
    .expect("update should succeed");

    let provider2 = MockIdentityProvider::new("mock");
    let audit = MockAuditLog::new();
    let audit_clone = audit.clone();
    let svc2 = make_service_with_audit(repo, provider2, make_config(), audit);

    let request = RefreshRequest {
        refresh_token,
        client_addr: oidc_exchange_core::domain::ClientAddr::Peer("203.0.113.22".parse().unwrap()),
        user_agent: Some("test-agent/3.0".to_string()),
        device_id: None,
    };
    svc2.refresh(request)
        .await
        .expect_err("refresh should fail for suspended user");

    let events = audit_clone.events().await;
    assert_eq!(
        events.len(),
        1,
        "suspended-user refresh must emit exactly one event, got: {:?}",
        events.iter().map(|e| &e.event_type).collect::<Vec<_>>()
    );
    assert_eq!(events[0].event_type, AuditEventType::UserSuspended);
    match &events[0].outcome {
        AuditOutcome::Failure(_) => {}
        other => panic!("expected Failure outcome, got: {:?}", other),
    }
    assert_eq!(events[0].actor.as_deref(), Some(user_id.as_str()));
    assert_eq!(events[0].ip_address.as_deref(), Some("203.0.113.22"));
    assert_eq!(events[0].user_agent.as_deref(), Some("test-agent/3.0"));
}

/// A successful refresh emits exactly one `TokenRefresh` (info, success)
/// event, carrying the user id as actor and the request's ip/ua — visible
/// under the default `info` `emit_threshold` since `TokenRefresh` is `Info`
/// severity.
#[tokio::test]
async fn refresh_success_emits_token_refresh_event() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let svc = make_service(repo.clone(), provider);

    let refresh_token = exchange_and_get_refresh_token(&repo, &svc).await;

    let users = repo.get_all_users().await;
    let user_id = users[0].id.clone();

    let provider2 = MockIdentityProvider::new("mock");
    let audit = MockAuditLog::new();
    let audit_clone = audit.clone();
    let svc2 = make_service_with_audit(repo, provider2, make_config(), audit);

    let request = RefreshRequest {
        refresh_token,
        client_addr: oidc_exchange_core::domain::ClientAddr::Peer("203.0.113.23".parse().unwrap()),
        user_agent: Some("test-agent/4.0".to_string()),
        device_id: None,
    };
    svc2.refresh(request).await.expect("refresh should succeed");

    let events = audit_clone.events().await;
    assert_eq!(
        events.len(),
        1,
        "successful refresh must emit exactly one event, got: {:?}",
        events.iter().map(|e| &e.event_type).collect::<Vec<_>>()
    );
    assert_eq!(events[0].event_type, AuditEventType::TokenRefresh);
    assert_eq!(events[0].outcome, AuditOutcome::Success);
    assert_eq!(events[0].actor.as_deref(), Some(user_id.as_str()));
    assert_eq!(events[0].ip_address.as_deref(), Some("203.0.113.23"));
    assert_eq!(events[0].user_agent.as_deref(), Some("test-agent/4.0"));
}

// ===========================================================================
// Task 07 — rotation flow branch matrix
//
// Every RefreshResolution branch, the grace boundary (deterministic via the
// mock's retirement backdating hook), expiry inheritance, CAS-loser
// behaviour, disabled-switch compatibility, legacy-row first redemption,
// write-ordering guarantees, and audit secrecy/severity.
// ===========================================================================

/// Hash a plaintext refresh token the way the service does.
fn hash_of(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// Hash a wrapped plaintext refresh token the way the service does.
fn hash_of_secret(token: &oidc_exchange_core::secret::Secret<String>) -> String {
    hash_of(token.expose())
}

/// Decode an access token JWT's payload into typed claims.
fn decode_claims(access_token: &str) -> AccessTokenClaims {
    let parts: Vec<&str> = access_token.split('.').collect();
    assert_eq!(parts.len(), 3, "JWT must have 3 parts");
    let payload = URL_SAFE_NO_PAD
        .decode(parts[1])
        .expect("payload should be valid base64url");
    serde_json::from_slice(&payload).expect("payload should deserialize as AccessTokenClaims")
}

/// The live session of the (single) family in the store.
async fn sole_live_session(repo: &MockRepository) -> Session {
    let sessions = repo.get_all_sessions().await;
    assert_eq!(sessions.len(), 1, "expected exactly one live generation");
    sessions.into_iter().next().expect("checked length above")
}

#[tokio::test]
async fn rotation_replaces_generation_and_inherits_family_metadata() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let svc = make_service(repo.clone(), provider);

    let refresh_token = exchange_and_get_refresh_token(&repo, &svc).await;
    let original = sole_live_session(&repo).await;

    let response = svc
        .refresh(RefreshRequest {
            refresh_token: refresh_token.clone(),
            client_addr: oidc_exchange_core::domain::ClientAddr::Peer(
                "203.0.113.30".parse().unwrap(),
            ),
            user_agent: Some("rotation-agent/1.0".to_string()),
            device_id: original.device_id.clone(),
        })
        .await
        .expect("rotation should succeed");

    // Replacement identity: fresh opaque token, same family, generation+1,
    // rotated_at set; everything identifying the sign-in inherited unchanged
    // — above all the absolute expires_at (the regression most likely to
    // arrive with any rotation fix).
    let replacement_hash = hash_of_secret(
        response
            .refresh_token
            .as_ref()
            .expect("rotation returns a replacement"),
    );
    assert!(
        replacement_hash != *original.refresh_token_hash.expose(),
        "replacement must be a new generation"
    );

    let replacement = sole_live_session(&repo).await;
    assert_eq!(replacement.family_id, original.family_id);
    assert_eq!(replacement.generation, original.generation + 1);
    assert_eq!(
        replacement.expires_at, original.expires_at,
        "absolute expiry must be inherited exactly"
    );
    assert_eq!(replacement.created_at, original.created_at);
    assert_eq!(replacement.user_id, original.user_id);
    assert_eq!(replacement.provider, original.provider);
    assert_eq!(replacement.device_id, original.device_id);
    assert_eq!(replacement.user_agent, original.user_agent);
    assert_eq!(replacement.ip_address, original.ip_address);
    assert!(replacement.rotated_at.is_some());
    assert!(is_valid_family_id(&replacement.family_id));

    // The presented hash is retired, not live, immediately after the swap.
    let presented_hash = hash_of(&refresh_token);
    match repo
        .resolve_refresh_token(&presented_hash)
        .await
        .expect("classify")
    {
        RefreshResolution::Superseded { live, .. } => {
            assert!(*live.refresh_token_hash.expose() == replacement_hash)
        }
        other => panic!("presented generation must be Superseded, got {other:?}"),
    }
}

/// Grace: presenting the immediately-preceding generation inside the window
/// rotates forward once more and raises no alarm — but only once: the same
/// presentation afterwards is reuse, because its successor is no longer live.
#[tokio::test]
async fn superseded_within_grace_rotates_forward_exactly_once() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let audit = MockAuditLog::new();
    let audit_clone = audit.clone();
    let svc = make_service_with_audit(repo.clone(), provider, make_config(), audit);

    let gen0_token = exchange_and_get_refresh_token(&repo, &svc).await;
    let gen1_response = svc
        .refresh(RefreshRequest {
            refresh_token: gen0_token.clone(),
            ..Default::default()
        })
        .await
        .expect("first rotation");
    let gen1_token = gen1_response
        .refresh_token
        .expect("first rotation issues gen 1")
        .into_inner();

    // Present gen 0 again immediately: inside the default 10s grace → rotates.
    let grace_response = svc
        .refresh(RefreshRequest {
            refresh_token: gen0_token.clone(),
            ..Default::default()
        })
        .await
        .expect("grace re-rotation should succeed once");
    let gen2_token = grace_response
        .refresh_token
        .expect("grace rotation issues gen 2")
        .into_inner();
    assert_ne!(gen2_token, gen1_token);

    // No reuse alarm fired for either redemption of gen 0.
    for event in audit_clone.events().await {
        assert_ne!(
            event.event_type,
            AuditEventType::RefreshTokenReuse,
            "grace rotations must never alarm"
        );
    }

    // Presenting gen 0 yet again is now reuse: its successor (gen 1) lost
    // liveness to the grace rotation. The whole family dies silently.
    let err = svc
        .refresh(RefreshRequest {
            refresh_token: gen0_token,
            ..Default::default()
        })
        .await
        .expect_err("third presentation of gen 0 is reuse");
    match err {
        Error::InvalidToken { reason } => {
            assert_eq!(
                reason, "unknown refresh token",
                "reuse hides behind the unknown-token reason"
            )
        }
        other => panic!("expected InvalidToken, got {other:?}"),
    }
    assert!(
        repo.get_all_sessions().await.is_empty(),
        "reuse must revoke the family's live generation"
    );
    assert!(
        repo.get_all_retired_tokens().await.is_empty(),
        "reuse must revoke every retained retirement record"
    );
}

/// Outside the grace window a superseded presentation is reuse: family
/// revoked before the Warning event, exact unknown-token reason returned.
#[tokio::test]
async fn superseded_outside_grace_revokes_family_and_audits_warning() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let audit = MockAuditLog::new();
    let audit_clone = audit.clone();
    let svc = make_service_with_audit(repo.clone(), provider, make_config(), audit);

    let gen0_token = exchange_and_get_refresh_token(&repo, &svc).await;
    let user_id = sole_live_session(&repo).await.user_id;
    let family_id = sole_live_session(&repo).await.family_id;

    svc.refresh(RefreshRequest {
        refresh_token: gen0_token.clone(),
        ..Default::default()
    })
    .await
    .expect("normal rotation");

    // Backdate gen 0's retirement record beyond the 10s grace window so the
    // negative boundary is deterministic (no sleeping).
    let presented_hash = hash_of(&gen0_token);
    assert!(
        repo.backdate_retirement(&presented_hash, 11).await,
        "the retirement record must exist to be backdated"
    );

    let baseline = audit_clone.events().await.len();
    let err = svc
        .refresh(RefreshRequest {
            refresh_token: gen0_token,
            client_addr: oidc_exchange_core::domain::ClientAddr::Peer(
                "203.0.113.31".parse().unwrap(),
            ),
            user_agent: Some("attacker/1.0".to_string()),
            ..Default::default()
        })
        .await
        .expect_err("out-of-grace presentation is reuse");

    match err {
        Error::InvalidToken { reason } => {
            assert_eq!(reason, "unknown refresh token");
        }
        other => panic!("expected InvalidToken, got {other:?}"),
    }

    // The family is gone — live generation and retained records alike.
    assert!(repo.get_all_sessions().await.is_empty());
    assert!(repo.get_all_retired_tokens().await.is_empty());

    // Exactly one new event: RefreshTokenReuse at Warning with
    // {family_id, sessions_revoked} and no digest-shaped detail value.
    let events = audit_clone.events().await;
    assert_eq!(events.len(), baseline + 1);
    let reuse = events.last().expect("event recorded");
    assert_eq!(reuse.event_type, AuditEventType::RefreshTokenReuse);
    assert_eq!(reuse.severity, AuditSeverity::Warning);
    assert_eq!(reuse.outcome, AuditOutcome::Success);
    assert_eq!(reuse.actor.as_deref(), Some(user_id.as_str()));
    assert_eq!(
        reuse.detail.get("family_id"),
        Some(&serde_json::json!(family_id))
    );
    let revoked = reuse
        .detail
        .get("sessions_revoked")
        .and_then(|v| v.as_u64())
        .expect("sessions_revoked must be a number");
    assert!(
        revoked >= 2,
        "live + retired generations must both count, got {revoked}"
    );
    for (key, value) in &reuse.detail {
        assert!(
            matches!(key.as_str(), "family_id" | "sessions_revoked"),
            "unexpected audit detail key {key:?}"
        );
        if key == "sessions_revoked" {
            assert!(value.is_u64(), "sessions_revoked must be a number");
        }
    }
    assert!(
        !reuse
            .detail
            .values()
            .any(|v| v.as_str().is_some_and(|s| s.len() == 64)),
        "audit detail must not carry a token-hash-shaped value: {:?}",
        reuse.detail
    );
}

/// Reuse revokes only the offending family: a sibling sign-in survives.
#[tokio::test]
async fn reuse_revokes_only_the_offending_family() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let svc = make_service(repo.clone(), provider);

    // Two independent families for the same user.
    let victim_token = exchange_and_get_refresh_token(&repo, &svc).await;
    let sibling_token = exchange_and_get_refresh_token(&repo, &svc).await;

    // Rotate the victim twice so its gen 0 resolves Retired (successor no
    // longer live) regardless of elapsed time.
    let gen1 = svc
        .refresh(RefreshRequest {
            refresh_token: victim_token.clone(),
            ..Default::default()
        })
        .await
        .expect("victim rotation 1")
        .refresh_token
        .expect("gen 1")
        .into_inner();
    svc.refresh(RefreshRequest {
        refresh_token: gen1,
        ..Default::default()
    })
    .await
    .expect("victim rotation 2");

    let err = svc
        .refresh(RefreshRequest {
            refresh_token: victim_token,
            ..Default::default()
        })
        .await
        .expect_err("retired presentation is reuse");
    assert!(matches!(err, Error::InvalidToken { .. }));

    // The victim family is gone entirely…
    let remaining_sessions = repo.get_all_sessions().await;
    assert_eq!(remaining_sessions.len(), 1, "only the sibling survives");
    // …while the sibling family keeps its live generation untouched.
    let sibling = sole_live_session(&repo).await;
    assert!(*sibling.refresh_token_hash.expose() == hash_of(&sibling_token));

    // And the sibling token still redeems normally afterwards.
    svc.refresh(RefreshRequest {
        refresh_token: sibling_token,
        ..Default::default()
    })
    .await
    .expect("sibling family must be unaffected by the other family's reuse");
}

/// A losing compare-and-swap (concurrent winner) refuses generically:
/// no revocation, no alarm, the winner's replacement stays live.
#[tokio::test]
async fn concurrent_loser_refuses_without_revocation_or_alarm() {
    // A store wrapper whose CAS always loses — deterministic loser branch.
    struct LosingCasRepo {
        inner: MockRepository,
    }

    #[async_trait::async_trait]
    impl UserRepository for LosingCasRepo {
        async fn get_user_by_id(
            &self,
            id: &str,
        ) -> Result<Option<oidc_exchange_core::domain::User>> {
            self.inner.get_user_by_id(id).await
        }
        async fn get_user_by_external_id(
            &self,
            external_id: &str,
            provider: &str,
        ) -> Result<Option<oidc_exchange_core::domain::User>> {
            self.inner
                .get_user_by_external_id(external_id, provider)
                .await
        }
        async fn create_user(
            &self,
            new_user: &oidc_exchange_core::domain::NewUser,
        ) -> Result<oidc_exchange_core::domain::User> {
            self.inner.create_user(new_user).await
        }
        async fn update_user(
            &self,
            user_id: &str,
            patch: &oidc_exchange_core::domain::UserPatch,
        ) -> Result<oidc_exchange_core::domain::User> {
            self.inner.update_user(user_id, patch).await
        }
        async fn delete_user(&self, user_id: &str) -> Result<()> {
            self.inner.delete_user(user_id).await
        }
        async fn count_by_status(&self) -> Result<HashMap<String, u64>> {
            self.inner.count_by_status().await
        }
        async fn list_users(
            &self,
            cursor: Option<&str>,
            limit: u32,
        ) -> Result<oidc_exchange_core::domain::UserPage> {
            self.inner.list_users(cursor, limit).await
        }
    }

    #[async_trait::async_trait]
    impl SessionRepository for LosingCasRepo {
        async fn put_single_use(
            &self,
            key: &str,
            expires_at: chrono::DateTime<Utc>,
        ) -> Result<bool> {
            self.inner.put_single_use(key, expires_at).await
        }
        async fn take_single_use(&self, key: &str) -> Result<bool> {
            self.inner.take_single_use(key).await
        }
        async fn store_refresh_token(&self, session: &Session) -> Result<()> {
            self.inner.store_refresh_token(session).await
        }
        async fn get_session_by_refresh_token(
            &self,
            hash: &oidc_exchange_core::secret::Secret<String>,
        ) -> Result<Option<Session>> {
            self.inner.get_session_by_refresh_token(hash).await
        }
        async fn resolve_refresh_token(&self, hash: &str) -> Result<RefreshResolution> {
            self.inner.resolve_refresh_token(hash).await
        }
        async fn rotate_refresh_token(&self, _live: &str, _repl: &Session) -> Result<bool> {
            // Deterministic race loss: another redemption won the CAS.
            Ok(false)
        }
        async fn revoke_session(
            &self,
            hash: &oidc_exchange_core::secret::Secret<String>,
        ) -> Result<()> {
            self.inner.revoke_session(hash).await
        }
        async fn revoke_family(&self, family_id: &str) -> Result<u64> {
            self.inner.revoke_family(family_id).await
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
    }

    fn make_losing_service(repo: MockRepository, provider: MockIdentityProvider) -> AppService {
        let provider_id = provider.provider_id().to_string();
        let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
        providers.insert(provider_id, Box::new(provider));
        AppService::new(
            Box::new(LosingCasRepo {
                inner: repo.clone(),
            }),
            Box::new(LosingCasRepo { inner: repo }),
            Box::new(MockKeyManager::new()),
            Box::new(MockAuditLog::new()),
            Box::new(MockUserSync::new()),
            Box::new(oidc_exchange_test_utils::MockRateLimiter::new()),
            providers,
            make_config(),
        )
    }

    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let setup = make_service(repo.clone(), provider);
    let gen0_token = exchange_and_get_refresh_token(&repo, &setup).await;
    let before_sessions = repo.get_all_sessions().await;
    let before_retired = repo.get_all_retired_tokens().await;

    let loser = make_losing_service(repo.clone(), MockIdentityProvider::new("mock"));
    let err = loser
        .refresh(RefreshRequest {
            refresh_token: gen0_token,
            ..Default::default()
        })
        .await
        .expect_err("a losing CAS refuses the request");
    assert!(matches!(err, Error::InvalidToken { .. }));

    // Byte-identical store: no revocation, no partial rotation, no records.
    assert_eq!(repo.get_all_sessions().await, before_sessions);
    assert_eq!(repo.get_all_retired_tokens().await, before_retired);
}

/// `refresh_rotation = false`: the presented token stays live, nothing is
/// minted or retired, and the response carries no refresh token.
#[tokio::test]
async fn rotation_disabled_preserves_legacy_reusable_behaviour() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let mut config = make_config();
    config.token.refresh_rotation = false;
    let audit = MockAuditLog::new();
    let audit_clone = audit.clone();
    let svc = make_service_with_audit(repo.clone(), provider, config, audit);

    let refresh_token = exchange_and_get_refresh_token(&repo, &svc).await;
    let original = sole_live_session(&repo).await;

    // Exchange itself never rotates, so this response legitimately carries a
    // token; it is the *refresh* grant that must not.
    let response = svc
        .refresh(RefreshRequest {
            refresh_token,
            ..Default::default()
        })
        .await
        .expect("disabled-mode refresh should succeed");

    assert!(
        response.refresh_token.is_none(),
        "the disabled refresh grant must not return a replacement"
    );
    assert!(!response.access_token.is_empty());

    // The same generation is still live under the same hash: reusable tokens.
    assert_eq!(
        repo.get_all_sessions().await,
        vec![original],
        "nothing may be minted or retired while rotation is off"
    );
    assert!(repo.get_all_retired_tokens().await.is_empty());

    // The success audit still fires exactly once (exchange-era events aside).
    let events = audit_clone.events().await;
    let refresh_events: Vec<_> = events
        .iter()
        .filter(|e| e.event_type == AuditEventType::TokenRefresh)
        .collect();
    assert_eq!(refresh_events.len(), 1);
    assert_eq!(refresh_events[0].outcome, AuditOutcome::Success);
}

/// While rotation is off, leftover retirement classifications are treated as
/// unknown: refused with no alarm and no family revocation.
#[tokio::test]
async fn rotation_disabled_treats_retired_classifications_as_unknown_silently() {
    let repo = MockRepository::new();
    // Rotation-enabled period: create a leftover retirement record.
    let setup = make_service(repo.clone(), MockIdentityProvider::new("mock"));
    let gen0_token = exchange_and_get_refresh_token(&repo, &setup).await;
    setup
        .refresh(RefreshRequest {
            refresh_token: gen0_token.clone(),
            ..Default::default()
        })
        .await
        .expect("rotation-era redemption");
    assert_eq!(repo.get_all_retired_tokens().await.len(), 1);

    // Switch off. Presenting the retired generation must refuse silently.
    let audit = MockAuditLog::new();
    let audit_clone = audit.clone();
    let mut config = make_config();
    config.token.refresh_rotation = false;
    let svc = make_service_with_audit(
        repo.clone(),
        MockIdentityProvider::new("mock"),
        config,
        audit,
    );

    let err = svc
        .refresh(RefreshRequest {
            refresh_token: gen0_token,
            ..Default::default()
        })
        .await
        .expect_err("retired generations are refused with rotation off");
    match err {
        Error::InvalidToken { reason } => assert_eq!(reason, "unknown refresh token"),
        other => panic!("expected InvalidToken, got {other:?}"),
    }

    // No alarm, and — crucially — no revocation: the successor stays live.
    let events = audit_clone.events().await;
    assert!(
        !events
            .iter()
            .any(|e| e.event_type == AuditEventType::RefreshTokenReuse),
        "rotation-off reuse hits must not alarm"
    );
    assert_eq!(
        repo.get_all_sessions().await.len(),
        1,
        "the family must NOT be revoked while rotation is off"
    );
    assert_eq!(repo.get_all_retired_tokens().await.len(), 1);
}

/// A pre-rotation legacy row (empty-string family sentinel) redeems once:
/// enabled rotation mints its family through the CAS without writing a
/// retirement record (there is no prior generation to detect reuse against),
/// and the old hash resolves Unknown afterwards.
#[tokio::test]
async fn legacy_row_first_redemption_mints_family_without_retirement_record() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let svc = make_service(repo.clone(), provider);

    // Seed a legacy row exactly as pre-rotation builds stored them, for a
    // real (active) user so the post-classification user gate passes.
    let created = repo
        .create_user(&oidc_exchange_core::domain::NewUser {
            external_id: "legacy-sub".to_string(),
            provider: "mock".to_string(),
            email: Some("legacy@example.com".to_string()),
            display_name: None,
        })
        .await
        .expect("seed legacy user");
    let legacy = Session {
        user_id: created.id.clone(),
        refresh_token_hash: oidc_exchange_core::secret::Secret::new(hash_of("legacy-opaque-token")),
        family_id: String::new(),
        generation: 0,
        provider: "mock".to_string(),
        expires_at: Utc::now() + Duration::hours(1),
        rotated_at: None,
        device_id: None,
        user_agent: None,
        ip_address: None,
        created_at: Utc::now(),
    };
    repo.store_refresh_token(&legacy)
        .await
        .expect("seed legacy row");

    let response = svc
        .refresh(RefreshRequest {
            refresh_token: "legacy-opaque-token".to_string(),
            ..Default::default()
        })
        .await
        .expect("legacy first redemption should succeed");

    let replacement = sole_live_session(&repo).await;
    assert!(
        *replacement.refresh_token_hash.expose()
            == hash_of_secret(
                response
                    .refresh_token
                    .as_ref()
                    .expect("enabled rotation returns a replacement"),
            )
    );
    assert!(replacement.refresh_token_hash != legacy.refresh_token_hash);
    // The caller minted the family: adapters never synthesize one.
    assert!(
        is_valid_family_id(&replacement.family_id),
        "first redemption must install a freshly-minted well-formed family id"
    );
    assert_eq!(replacement.generation, 1);
    assert_eq!(replacement.expires_at, legacy.expires_at);
    assert_eq!(replacement.created_at, legacy.created_at);

    // No retirement record for the one transition that cannot be detected.
    assert!(
        repo.get_all_retired_tokens().await.is_empty(),
        "the legacy transition writes no retirement record"
    );

    // The old hash is simply gone.
    let resolution = repo
        .resolve_refresh_token(legacy.refresh_token_hash.expose())
        .await
        .expect("classify old hash");
    assert!(matches!(resolution, RefreshResolution::Unknown));
}

/// Expiry gate runs before any write: an expired live generation is refused
/// with the expired reason and the store is untouched.
#[tokio::test]
async fn expired_live_generation_is_refused_before_any_write() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let svc = make_service(repo.clone(), provider);

    let refresh_token = exchange_and_get_refresh_token(&repo, &svc).await;
    let original = sole_live_session(&repo).await;

    // Rewrite the live row in place with a past expiry (same hash).
    repo.revoke_session(&original.refresh_token_hash)
        .await
        .expect("swap out live row");
    let expired = Session {
        expires_at: Utc::now() - Duration::hours(1),
        ..original.clone()
    };
    repo.store_refresh_token(&expired)
        .await
        .expect("store expired row");

    let err = svc
        .refresh(RefreshRequest {
            refresh_token,
            ..Default::default()
        })
        .await
        .expect_err("expired generation must be refused");
    match err {
        Error::InvalidToken { reason } => assert_eq!(reason, "refresh token expired"),
        other => panic!("expected InvalidToken, got {other:?}"),
    }

    // Nothing was written: the expired row is still the only state.
    assert_eq!(
        repo.get_all_sessions().await,
        vec![expired],
        "expiry refusal must not rotate or retire anything"
    );
    assert!(repo.get_all_retired_tokens().await.is_empty());
}

/// A missing user is turned away before any write (wrapper repo hides the
/// user after the session exists).
#[tokio::test]
async fn missing_user_is_refused_before_any_write() {
    struct UserlessRepo {
        inner: MockRepository,
    }

    #[async_trait::async_trait]
    impl UserRepository for UserlessRepo {
        async fn get_user_by_id(
            &self,
            _user_id: &str,
        ) -> Result<Option<oidc_exchange_core::domain::User>> {
            Ok(None)
        }
        async fn get_user_by_external_id(
            &self,
            _external_id: &str,
            _provider: &str,
        ) -> Result<Option<oidc_exchange_core::domain::User>> {
            Ok(None)
        }
        async fn create_user(
            &self,
            new_user: &oidc_exchange_core::domain::NewUser,
        ) -> Result<oidc_exchange_core::domain::User> {
            self.inner.create_user(new_user).await
        }
        async fn update_user(
            &self,
            user_id: &str,
            patch: &oidc_exchange_core::domain::UserPatch,
        ) -> Result<oidc_exchange_core::domain::User> {
            self.inner.update_user(user_id, patch).await
        }
        async fn delete_user(&self, user_id: &str) -> Result<()> {
            self.inner.delete_user(user_id).await
        }
        async fn count_by_status(&self) -> Result<HashMap<String, u64>> {
            self.inner.count_by_status().await
        }
        async fn list_users(
            &self,
            cursor: Option<&str>,
            limit: u32,
        ) -> Result<oidc_exchange_core::domain::UserPage> {
            self.inner.list_users(cursor, limit).await
        }
    }

    #[async_trait::async_trait]
    impl SessionRepository for UserlessRepo {
        async fn put_single_use(
            &self,
            key: &str,
            expires_at: chrono::DateTime<Utc>,
        ) -> Result<bool> {
            self.inner.put_single_use(key, expires_at).await
        }
        async fn take_single_use(&self, key: &str) -> Result<bool> {
            self.inner.take_single_use(key).await
        }
        async fn store_refresh_token(&self, session: &Session) -> Result<()> {
            self.inner.store_refresh_token(session).await
        }
        async fn get_session_by_refresh_token(
            &self,
            hash: &oidc_exchange_core::secret::Secret<String>,
        ) -> Result<Option<Session>> {
            self.inner.get_session_by_refresh_token(hash).await
        }
        async fn resolve_refresh_token(&self, hash: &str) -> Result<RefreshResolution> {
            self.inner.resolve_refresh_token(hash).await
        }
        async fn rotate_refresh_token(&self, live: &str, repl: &Session) -> Result<bool> {
            self.inner.rotate_refresh_token(live, repl).await
        }
        async fn revoke_session(
            &self,
            hash: &oidc_exchange_core::secret::Secret<String>,
        ) -> Result<()> {
            self.inner.revoke_session(hash).await
        }
        async fn revoke_family(&self, family_id: &str) -> Result<u64> {
            self.inner.revoke_family(family_id).await
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
    }

    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let setup = make_service(repo.clone(), provider);
    let refresh_token = exchange_and_get_refresh_token(&repo, &setup).await;
    let before_sessions = repo.get_all_sessions().await;
    let before_retired = repo.get_all_retired_tokens().await;

    // Refresh never resolves a provider map entry, so an empty map is fine
    // for this service instance (only exchange paths look providers up).
    let svc = AppService::new(
        Box::new(UserlessRepo {
            inner: repo.clone(),
        }),
        Box::new(UserlessRepo {
            inner: repo.clone(),
        }),
        Box::new(MockKeyManager::new()),
        Box::new(MockAuditLog::new()),
        Box::new(MockUserSync::new()),
        Box::new(oidc_exchange_test_utils::MockRateLimiter::new()),
        HashMap::new(),
        make_config(),
    );

    let err = svc
        .refresh(RefreshRequest {
            refresh_token,
            ..Default::default()
        })
        .await
        .expect_err("missing user must refuse");
    match err {
        Error::InvalidToken { reason } => assert_eq!(reason, "user not found"),
        other => panic!("expected InvalidToken, got {other:?}"),
    }

    assert_eq!(repo.get_all_sessions().await, before_sessions);
    assert_eq!(repo.get_all_retired_tokens().await, before_retired);
}

/// Suspended users are turned away before any write — the error is
/// `UserSuspended`, and the store keeps the presented generation live.
#[tokio::test]
async fn suspended_user_writes_nothing_before_refusal() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let svc = make_service(repo.clone(), provider);

    let refresh_token = exchange_and_get_refresh_token(&repo, &svc).await;
    let original = sole_live_session(&repo).await;

    let users = repo.get_all_users().await;
    repo.update_user(
        &users[0].id,
        &UserPatch {
            status: Some(UserStatus::Suspended),
            email: None,
            display_name: None,
            metadata: None,
            claims: None,
        },
    )
    .await
    .expect("suspend user");

    let err = svc
        .refresh(RefreshRequest {
            refresh_token,
            ..Default::default()
        })
        .await
        .expect_err("suspended user must be refused");
    assert!(matches!(err, Error::UserSuspended { .. }));

    // Ordering proof: the presented generation is still live, untouched.
    assert_eq!(
        repo.get_all_sessions().await,
        vec![original],
        "suspension must be decided before any store write"
    );
    assert!(repo.get_all_retired_tokens().await.is_empty());
}

/// The successful-refresh audit carries `{family_id, generation, grace}` and
/// never any digest-shaped value; grace rotations flag themselves.
#[tokio::test]
async fn success_audit_carries_family_generation_grace_and_no_digest() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let audit = MockAuditLog::new();
    let audit_clone = audit.clone();
    let svc = make_service_with_audit(repo.clone(), provider, make_config(), audit);

    let gen0_token = exchange_and_get_refresh_token(&repo, &svc).await;
    let family_id = sole_live_session(&repo).await.family_id;

    // Normal rotation → grace: false.
    let gen1_token = svc
        .refresh(RefreshRequest {
            refresh_token: gen0_token.clone(),
            ..Default::default()
        })
        .await
        .expect("normal rotation")
        .refresh_token
        .expect("gen 1")
        .into_inner();

    // Grace rotation → grace: true.
    svc.refresh(RefreshRequest {
        refresh_token: gen0_token,
        ..Default::default()
    })
    .await
    .expect("grace rotation");

    let events = audit_clone.events().await;
    let refreshes: Vec<_> = events
        .iter()
        .filter(|e| e.event_type == AuditEventType::TokenRefresh)
        .collect();
    assert_eq!(refreshes.len(), 2, "exactly two successful refreshes");
    assert_eq!(refreshes[0].severity, AuditSeverity::Info);
    assert_eq!(refreshes[0].outcome, AuditOutcome::Success);
    assert_eq!(
        refreshes[0].detail.get("grace"),
        Some(&serde_json::json!(false)),
        "normal rotation is not via grace"
    );
    assert_eq!(
        refreshes[0].detail.get("family_id"),
        Some(&serde_json::json!(family_id))
    );
    assert_eq!(
        refreshes[0].detail.get("generation"),
        Some(&serde_json::json!(1))
    );
    assert_eq!(
        refreshes[1].detail.get("grace"),
        Some(&serde_json::json!(true)),
        "the second redemption of gen 0 came through the grace window"
    );
    assert_eq!(
        refreshes[1].detail.get("generation"),
        Some(&serde_json::json!(2))
    );

    // Secrecy: no digest-shaped string anywhere in either event's detail.
    for event in &refreshes {
        assert!(
            !event
                .detail
                .values()
                .any(|v| v.as_str().is_some_and(|s| s.len() == 64)),
            "audit detail must never carry a token-hash-shaped value: {:?}",
            event.detail
        );
    }
    // Silence check on gen1's plaintext too: it must appear nowhere.
    let serialized = format!("{:?}", refreshes);
    assert!(
        !serialized.contains(gen1_token.as_str()),
        "replacement token plaintext must never reach audit events"
    );
}
