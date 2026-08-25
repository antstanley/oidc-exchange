use std::collections::HashMap;

use serde_json::json;

use oidc_exchange_core::config::{
    Config, RawAuditConfig, RawConfig, RawRegistrationConfig, RawServerConfig, RawTelemetryConfig,
    RawTokenConfig,
};
use oidc_exchange_core::domain::{
    AuditEventType, NewUser, OperatorAuthMechanism, OperatorPrincipal, UserPatch, UserStatus,
};
use oidc_exchange_core::error::Error;
use oidc_exchange_core::ports::{IdentityProvider, UserRepository};
use oidc_exchange_core::service::exchange::{ExchangeCredential, ExchangeRequest};
use oidc_exchange_core::service::AppService;

use oidc_exchange_test_utils::{
    MockAuditLog, MockIdentityProvider, MockKeyManager, MockRateLimiter, MockRepository,
    MockUserSync, UserSyncCall,
};

/// The principal every direct-service test acts as. Attribution assertions
/// read this same shape back out of recorded audit events.
fn operator() -> OperatorPrincipal {
    OperatorPrincipal {
        id: "usr_operator_test".to_string(),
        mechanism: OperatorAuthMechanism::OperatorToken,
    }
}

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
fn make_service_with_mocks(
    repo: MockRepository,
    user_sync: MockUserSync,
) -> (AppService, MockRepository, MockUserSync) {
    let provider = MockIdentityProvider::new("mock");
    let provider_id = provider.provider_id().to_string();
    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    providers.insert(provider_id, Box::new(provider));

    let repo_clone = repo.clone();
    let sync_clone = user_sync.clone();

    let svc = AppService::new(
        Box::new(repo.clone()),
        Box::new(repo),
        Box::new(MockKeyManager::new()),
        Box::new(MockAuditLog::new()),
        Box::new(user_sync),
        Box::new(MockRateLimiter::new()),
        providers,
        make_config(),
    );

    (svc, repo_clone, sync_clone)
}

fn make_service_with_provider(
    repo: MockRepository,
    user_sync: MockUserSync,
    provider: MockIdentityProvider,
) -> AppService {
    let provider_id = provider.provider_id().to_string();
    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    providers.insert(provider_id, Box::new(provider));

    AppService::new(
        Box::new(repo.clone()),
        Box::new(repo),
        Box::new(MockKeyManager::new()),
        Box::new(MockAuditLog::new()),
        Box::new(user_sync),
        Box::new(MockRateLimiter::new()),
        providers,
        make_config(),
    )
}

/// Wire a service against an explicitly supplied `MockAuditLog` (and config)
/// so tests can inspect emitted audit events or force adapter failures.
fn make_service_with_audit(
    repo: MockRepository,
    user_sync: MockUserSync,
    audit: MockAuditLog,
    config: Config,
) -> AppService {
    let provider = MockIdentityProvider::new("mock");
    let provider_id = provider.provider_id().to_string();
    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    providers.insert(provider_id, Box::new(provider));

    AppService::new(
        Box::new(repo.clone()),
        Box::new(repo),
        Box::new(MockKeyManager::new()),
        Box::new(audit),
        Box::new(user_sync),
        Box::new(MockRateLimiter::new()),
        providers,
        config,
    )
}

fn new_user(ext_id: &str, provider: &str) -> NewUser {
    NewUser {
        external_id: ext_id.to_string(),
        provider: provider.to_string(),
        email: Some(format!("{}@example.com", ext_id)),
        display_name: Some(format!("User {}", ext_id)),
    }
}

// ─── Test 1: Create user via admin ──────────────────────────────────────────

#[tokio::test]
async fn admin_create_user_triggers_sync() {
    let repo = MockRepository::new();
    let user_sync = MockUserSync::new();
    let (svc, repo_clone, sync_clone) = make_service_with_mocks(repo, user_sync);

    let nu = new_user("ext-1", "google");
    let user = svc
        .admin_create_user(&operator(), &nu)
        .await
        .expect("create should succeed");

    // Verify user in repo
    let stored = repo_clone.get_all_users().await;
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].id, user.id);
    assert_eq!(stored[0].external_id, "ext-1");
    assert_eq!(stored[0].status, UserStatus::Active);

    // Verify sync call recorded
    let calls = sync_clone.calls().await;
    assert_eq!(calls.len(), 1);
    match &calls[0] {
        UserSyncCall::Created(u) => {
            assert_eq!(u.id, user.id);
        }
        other => panic!("expected Created, got {:?}", other),
    }
}

// ─── Test 2: Update user with partial patch ─────────────────────────────────

#[tokio::test]
async fn admin_update_user_partial_patch_reports_changed_fields() {
    let repo = MockRepository::new();
    let user_sync = MockUserSync::new();
    let (svc, _repo_clone, sync_clone) = make_service_with_mocks(repo, user_sync);

    // Create a user first
    let nu = new_user("ext-2", "google");
    let user = svc
        .admin_create_user(&operator(), &nu)
        .await
        .expect("create should succeed");

    // Update only email
    let patch = UserPatch {
        email: Some("new-email@example.com".to_string()),
        display_name: None,
        metadata: None,
        claims: None,
        status: None,
    };
    let updated = svc
        .admin_update_user(&operator(), &user.id, &patch)
        .await
        .expect("update should succeed");

    assert_eq!(updated.email.as_deref(), Some("new-email@example.com"));

    // Verify sync call: should have ["email"] as changed_fields
    let calls = sync_clone.calls().await;
    // First call is Created from admin_create_user, second is Updated
    assert_eq!(calls.len(), 2);
    match &calls[1] {
        UserSyncCall::Updated {
            user: u,
            changed_fields,
        } => {
            assert_eq!(u.id, user.id);
            assert_eq!(changed_fields, &["email".to_string()]);
        }
        other => panic!("expected Updated, got {:?}", other),
    }
}

// ─── Test 3: Merge claims ───────────────────────────────────────────────────

#[tokio::test]
async fn admin_merge_claims_preserves_existing() {
    let repo = MockRepository::new();
    let user_sync = MockUserSync::new();
    let (svc, _repo_clone, _sync_clone) = make_service_with_mocks(repo, user_sync);

    // Create user
    let nu = new_user("ext-3", "google");
    let user = svc
        .admin_create_user(&operator(), &nu)
        .await
        .expect("create should succeed");

    // Set initial claims {"a": 1}
    let mut initial = HashMap::new();
    initial.insert("a".to_string(), json!(1));
    svc.admin_set_claims(&operator(), &user.id, initial)
        .await
        .expect("set claims should succeed");

    // Merge {"b": 2}
    let mut merge = HashMap::new();
    merge.insert("b".to_string(), json!(2));
    svc.admin_merge_claims(&operator(), &user.id, merge)
        .await
        .expect("merge claims should succeed");

    // Verify result is {"a": 1, "b": 2}
    let claims = svc
        .admin_get_claims(&user.id)
        .await
        .expect("get claims should succeed");
    assert_eq!(claims.get("a"), Some(&json!(1)));
    assert_eq!(claims.get("b"), Some(&json!(2)));
    assert_eq!(claims.len(), 2);
}

// ─── Test 4: Set claims replaces entirely ───────────────────────────────────

#[tokio::test]
async fn admin_set_claims_replaces_entirely() {
    let repo = MockRepository::new();
    let user_sync = MockUserSync::new();
    let (svc, _repo_clone, _sync_clone) = make_service_with_mocks(repo, user_sync);

    // Create user
    let nu = new_user("ext-4", "google");
    let user = svc
        .admin_create_user(&operator(), &nu)
        .await
        .expect("create should succeed");

    // Set initial claims {"a": 1, "b": 2}
    let mut initial = HashMap::new();
    initial.insert("a".to_string(), json!(1));
    initial.insert("b".to_string(), json!(2));
    svc.admin_set_claims(&operator(), &user.id, initial)
        .await
        .expect("set claims should succeed");

    // Replace with {"c": 3}
    let mut replacement = HashMap::new();
    replacement.insert("c".to_string(), json!(3));
    svc.admin_set_claims(&operator(), &user.id, replacement)
        .await
        .expect("set claims should succeed");

    // Verify result is {"c": 3} only
    let claims = svc
        .admin_get_claims(&user.id)
        .await
        .expect("get claims should succeed");
    assert_eq!(claims.get("c"), Some(&json!(3)));
    assert_eq!(claims.len(), 1);
    assert!(!claims.contains_key("a"));
    assert!(!claims.contains_key("b"));
}

// ─── Test 5: Clear claims ───────────────────────────────────────────────────

#[tokio::test]
async fn admin_clear_claims_empties_map() {
    let repo = MockRepository::new();
    let user_sync = MockUserSync::new();
    let (svc, _repo_clone, _sync_clone) = make_service_with_mocks(repo, user_sync);

    // Create user
    let nu = new_user("ext-5", "google");
    let user = svc
        .admin_create_user(&operator(), &nu)
        .await
        .expect("create should succeed");

    // Set some claims
    let mut initial = HashMap::new();
    initial.insert("x".to_string(), json!("hello"));
    initial.insert("y".to_string(), json!(42));
    svc.admin_set_claims(&operator(), &user.id, initial)
        .await
        .expect("set claims should succeed");

    // Clear
    svc.admin_clear_claims(&operator(), &user.id)
        .await
        .expect("clear claims should succeed");

    // Verify empty
    let claims = svc
        .admin_get_claims(&user.id)
        .await
        .expect("get claims should succeed");
    assert!(claims.is_empty(), "claims should be empty after clear");
}

// ─── Closed reserved-claim enforcement at every write boundary ──────────────

/// The exact 24-name closed set, spelled independently of
/// `RESERVED_CLAIMS` so drift on either side fails these tests.
const RESERVED_CLAIM_NAMES: [&str; 24] = [
    "iss",
    "sub",
    "aud",
    "exp",
    "nbf",
    "iat",
    "jti",
    "acr",
    "amr",
    "at_hash",
    "auth_time",
    "azp",
    "c_hash",
    "cnf",
    "nonce",
    "sid",
    "typ",
    "client_id",
    "scope",
    "scp",
    "roles",
    "groups",
    "entitlements",
    "permissions",
];

#[tokio::test]
async fn admin_set_claims_rejects_every_reserved_claim_name() {
    let repo = MockRepository::new();
    let user_sync = MockUserSync::new();
    let (svc, _repo_clone, _sync_clone) = make_service_with_mocks(repo, user_sync);

    let nu = new_user("ext-reserved-set", "google");
    let user = svc
        .admin_create_user(&operator(), &nu)
        .await
        .expect("create should succeed");

    for name in RESERVED_CLAIM_NAMES {
        let mut claims = HashMap::new();
        claims.insert(name.to_string(), json!("override"));
        // A second, legitimate key proves rejection is caused by the reserved
        // name and not by the payload shape.
        claims.insert("tenant".to_string(), json!("acme"));

        let err = svc
            .admin_set_claims(&operator(), &user.id, claims)
            .await
            .expect_err("a reserved claim name must be rejected by set");
        assert!(
            matches!(&err, Error::InvalidRequest { .. }),
            "{name:?} must map to InvalidRequest, got {err:?}"
        );
        match &err {
            Error::InvalidRequest { reason } => {
                assert!(
                    reason.contains(&format!("\"{name}\"")),
                    "the error must name the offending key {name:?}: {reason}"
                );
                assert!(
                    reason.contains("reserved"),
                    "the error must state the reservation: {reason}"
                );
            }
            other => panic!("expected InvalidRequest, got {other:?}"),
        }
    }

    // Nothing from any rejected payload may have been persisted.
    let stored = svc
        .admin_get_claims(&user.id)
        .await
        .expect("get claims should succeed");
    assert!(
        stored.is_empty(),
        "rejected set payloads must not reach persistence, got {stored:?}"
    );
}

#[tokio::test]
async fn admin_set_claims_still_accepts_and_persists_non_reserved_names() {
    let repo = MockRepository::new();
    let user_sync = MockUserSync::new();
    let (svc, _repo_clone, _sync_clone) = make_service_with_mocks(repo, user_sync);

    let nu = new_user("ext-reserved-set-ok", "google");
    let user = svc
        .admin_create_user(&operator(), &nu)
        .await
        .expect("create should succeed");

    let mut claims = HashMap::new();
    claims.insert("role".to_string(), json!("admin"));
    claims.insert("Sub".to_string(), json!("case-sensitive"));
    claims.insert("tenant".to_string(), json!("acme"));
    svc.admin_set_claims(&operator(), &user.id, claims)
        .await
        .expect("non-reserved names must be accepted");

    let stored = svc
        .admin_get_claims(&user.id)
        .await
        .expect("get claims should succeed");
    assert_eq!(stored.get("role"), Some(&json!("admin")));
    assert_eq!(stored.get("Sub"), Some(&json!("case-sensitive")));
    assert_eq!(stored.len(), 3);
}

#[tokio::test]
async fn admin_merge_claims_rejects_reserved_delta_but_preserves_existing() {
    let repo = MockRepository::new();
    let user_sync = MockUserSync::new();
    let (svc, _repo_clone, _sync_clone) = make_service_with_mocks(repo, user_sync);

    let nu = new_user("ext-reserved-merge", "google");
    let user = svc
        .admin_create_user(&operator(), &nu)
        .await
        .expect("create should succeed");

    let mut seed = HashMap::new();
    seed.insert("tier".to_string(), json!("gold"));
    svc.admin_set_claims(&operator(), &user.id, seed)
        .await
        .expect("seed set should succeed");

    for name in RESERVED_CLAIM_NAMES {
        let mut delta = HashMap::new();
        delta.insert(name.to_string(), json!("forged"));

        let err = svc
            .admin_merge_claims(&operator(), &user.id, delta)
            .await
            .expect_err("a reserved claim name must be rejected by merge");
        assert!(
            matches!(&err, Error::InvalidRequest { .. }),
            "{name:?} must map to InvalidRequest, got {err:?}"
        );

        // The stored map must survive every rejected merge untouched.
        let stored = svc
            .admin_get_claims(&user.id)
            .await
            .expect("get claims should succeed");
        assert_eq!(
            stored.get("tier"),
            Some(&json!("gold")),
            "existing claims must survive a rejected merge of {name:?}"
        );
        assert!(
            !stored.contains_key(name),
            "the reserved name {name:?} must never land in storage"
        );
    }
}

#[tokio::test]
async fn admin_update_user_rejects_reserved_names_in_claims_patch() {
    let repo = MockRepository::new();
    let user_sync = MockUserSync::new();
    let (svc, _repo_clone, _sync_clone) = make_service_with_mocks(repo, user_sync);

    let nu = new_user("ext-reserved-patch", "google");
    let user = svc
        .admin_create_user(&operator(), &nu)
        .await
        .expect("create should succeed");

    for name in RESERVED_CLAIM_NAMES {
        let mut patch_claims = HashMap::new();
        patch_claims.insert(name.to_string(), json!("override"));

        let patch = UserPatch {
            email: None,
            display_name: None,
            metadata: None,
            claims: Some(patch_claims),
            status: None,
        };
        let err = svc
            .admin_update_user(&operator(), &user.id, &patch)
            .await
            .expect_err("a reserved claim name must be rejected by update");
        assert!(
            matches!(&err, Error::InvalidRequest { reason } if reason.contains(&format!("\"{name}\""))),
            "{name:?} must be named by an InvalidRequest, got {err:?}"
        );
    }

    // The version counter is store-managed evidence that no write happened.
    let after = svc
        .admin_get_user(&user.id)
        .await
        .expect("get user should succeed")
        .expect("user should still exist");
    assert_eq!(
        after.version,
        oidc_exchange_core::domain::INITIAL_USER_VERSION,
        "no rejected patch may have reached the store"
    );
    let stored = svc
        .admin_get_claims(&user.id)
        .await
        .expect("get claims should succeed");
    assert!(stored.is_empty(), "no rejected patch claims may persist");
}

#[tokio::test]
async fn admin_update_user_accepts_non_reserved_claims_patch() {
    let repo = MockRepository::new();
    let user_sync = MockUserSync::new();
    let (svc, _repo_clone, _sync_clone) = make_service_with_mocks(repo, user_sync);

    let nu = new_user("ext-patch-ok", "google");
    let user = svc
        .admin_create_user(&operator(), &nu)
        .await
        .expect("create should succeed");

    let mut patch_claims = HashMap::new();
    patch_claims.insert("entitlements_custom".to_string(), json!(true));
    patch_claims.insert("groups_v2".to_string(), json!(["team-a"]));
    let patch = UserPatch {
        email: None,
        display_name: None,
        metadata: None,
        claims: Some(patch_claims),
        status: None,
    };
    svc.admin_update_user(&operator(), &user.id, &patch)
        .await
        .expect("a non-reserved claims patch must apply");

    let stored = svc
        .admin_get_claims(&user.id)
        .await
        .expect("get claims should succeed");
    assert_eq!(stored.get("groups_v2"), Some(&json!(["team-a"])));
    assert_eq!(stored.len(), 2);
}

// ─── Test 5b: Claims operations on unknown user id return NotFound ─────────

#[tokio::test]
async fn admin_get_claims_unknown_id_returns_not_found() {
    let repo = MockRepository::new();
    let user_sync = MockUserSync::new();
    let (svc, _repo_clone, _sync_clone) = make_service_with_mocks(repo, user_sync);

    let result = svc.admin_get_claims("usr_does_not_exist").await;
    assert!(matches!(result, Err(Error::NotFound { .. })));
    assert!(
        !matches!(result, Err(Error::InvalidRequest { .. })),
        "unknown id must not surface as InvalidRequest"
    );
}

#[tokio::test]
async fn admin_set_claims_unknown_id_returns_not_found() {
    let repo = MockRepository::new();
    let user_sync = MockUserSync::new();
    let (svc, repo_clone, _sync_clone) = make_service_with_mocks(repo, user_sync);

    let mut claims = HashMap::new();
    claims.insert("a".to_string(), json!(1));
    let result = svc
        .admin_set_claims(&operator(), "usr_does_not_exist", claims)
        .await;
    assert!(matches!(result, Err(Error::NotFound { .. })));

    // The rejected set must not have written a user row.
    assert!(repo_clone.get_all_users().await.is_empty());
}

#[tokio::test]
async fn admin_merge_claims_unknown_id_returns_not_found() {
    let repo = MockRepository::new();
    let user_sync = MockUserSync::new();
    let (svc, repo_clone, _sync_clone) = make_service_with_mocks(repo, user_sync);

    let mut claims = HashMap::new();
    claims.insert("a".to_string(), json!(1));
    let result = svc
        .admin_merge_claims(&operator(), "usr_does_not_exist", claims)
        .await;
    assert!(matches!(result, Err(Error::NotFound { .. })));

    // The rejected merge must not have written a user row.
    assert!(repo_clone.get_all_users().await.is_empty());
}

#[tokio::test]
async fn admin_clear_claims_unknown_id_returns_not_found() {
    let repo = MockRepository::new();
    let user_sync = MockUserSync::new();
    let (svc, repo_clone, _sync_clone) = make_service_with_mocks(repo, user_sync);

    let result = svc
        .admin_clear_claims(&operator(), "usr_does_not_exist")
        .await;
    assert!(matches!(result, Err(Error::NotFound { .. })));

    // The rejected clear must not have written a user row.
    assert!(repo_clone.get_all_users().await.is_empty());
}

// ─── Test 6: Delete user revokes sessions ───────────────────────────────────

#[tokio::test]
async fn admin_delete_user_revokes_sessions() {
    let repo = MockRepository::new();
    let user_sync = MockUserSync::new();
    let provider = MockIdentityProvider::new("mock");
    let repo_clone = repo.clone();
    let sync_clone = user_sync.clone();
    let svc = make_service_with_provider(repo, user_sync, provider);

    // Exchange to create a user + session
    let request = ExchangeRequest {
        provider_access_token: None,
        credential: ExchangeCredential::AuthorizationCode {
            code: "auth-code".to_string(),
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
    assert!(response.refresh_token.is_some());

    // Verify session exists
    let sessions = repo_clone.get_all_sessions().await;
    assert_eq!(sessions.len(), 1);
    let user_id = sessions[0].user_id.clone();

    // Delete the user via admin
    svc.admin_delete_user(&operator(), &user_id)
        .await
        .expect("delete should succeed");

    // Verify user status is Deleted
    let users = repo_clone.get_all_users().await;
    let user = users
        .iter()
        .find(|u| u.id == user_id)
        .expect("user should exist");
    assert_eq!(user.status, UserStatus::Deleted);

    // Verify all sessions revoked
    let sessions = repo_clone.get_all_sessions().await;
    assert!(
        sessions.is_empty(),
        "all sessions should be revoked after delete"
    );

    // Verify sync call
    let calls = sync_clone.calls().await;
    let has_deleted = calls
        .iter()
        .any(|c| matches!(c, UserSyncCall::Deleted(id) if id == &user_id));
    assert!(has_deleted, "should have a Deleted sync call for the user");
}

// ─── Lifecycle enforcement in admin_update_user / admin_delete_user ────────

fn status_patch(status: UserStatus) -> UserPatch {
    UserPatch {
        email: None,
        display_name: None,
        metadata: None,
        claims: None,
        status: Some(status),
    }
}

/// Creates a user + a live refresh-token session for it via the exchange flow,
/// returning the service (with mock repo/sync attached), the user id, and the
/// cloned mock repo/sync handles for direct inspection.
async fn service_with_active_session() -> (AppService, String, MockRepository, MockUserSync) {
    let repo = MockRepository::new();
    let user_sync = MockUserSync::new();
    let provider = MockIdentityProvider::new("mock");
    let repo_clone = repo.clone();
    let sync_clone = user_sync.clone();
    let svc = make_service_with_provider(repo, user_sync, provider);

    let request = ExchangeRequest {
        provider_access_token: None,
        credential: ExchangeCredential::AuthorizationCode {
            code: "auth-code".to_string(),
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
    assert!(response.refresh_token.is_some());

    let sessions = repo_clone.get_all_sessions().await;
    assert_eq!(
        sessions.len(),
        1,
        "exchange should create exactly one session"
    );
    let user_id = sessions[0].user_id.clone();

    (svc, user_id, repo_clone, sync_clone)
}

#[tokio::test]
async fn patch_to_suspended_revokes_sessions() {
    let (svc, user_id, repo_clone, _sync_clone) = service_with_active_session().await;

    let updated = svc
        .admin_update_user(&operator(), &user_id, &status_patch(UserStatus::Suspended))
        .await
        .expect("suspend patch should succeed");
    assert_eq!(updated.status, UserStatus::Suspended);

    let sessions = repo_clone.get_all_sessions().await;
    assert!(
        sessions.is_empty(),
        "sessions should be revoked on transition into Suspended"
    );
}

#[tokio::test]
async fn patch_to_deleted_revokes_sessions() {
    let (svc, user_id, repo_clone, _sync_clone) = service_with_active_session().await;

    let updated = svc
        .admin_update_user(&operator(), &user_id, &status_patch(UserStatus::Deleted))
        .await
        .expect("delete patch should succeed");
    assert_eq!(updated.status, UserStatus::Deleted);

    let sessions = repo_clone.get_all_sessions().await;
    assert!(
        sessions.is_empty(),
        "sessions should be revoked on transition into Deleted"
    );
}

#[tokio::test]
async fn reactivated_user_has_no_surviving_sessions() {
    let (svc, user_id, repo_clone, _sync_clone) = service_with_active_session().await;

    svc.admin_update_user(&operator(), &user_id, &status_patch(UserStatus::Suspended))
        .await
        .expect("suspend should succeed");
    assert!(repo_clone.get_all_sessions().await.is_empty());

    let reactivated = svc
        .admin_update_user(&operator(), &user_id, &status_patch(UserStatus::Active))
        .await
        .expect("reactivation should succeed");
    assert_eq!(reactivated.status, UserStatus::Active);

    let sessions = repo_clone.get_all_sessions().await;
    assert!(
        sessions.is_empty(),
        "reactivation must not resurrect the sessions killed by suspension"
    );
}

#[tokio::test]
async fn suspend_then_delete_succeeds_and_leaves_user_deleted() {
    let (svc, user_id, repo_clone, _sync_clone) = service_with_active_session().await;

    svc.admin_update_user(&operator(), &user_id, &status_patch(UserStatus::Suspended))
        .await
        .expect("suspend should succeed");

    svc.admin_delete_user(&operator(), &user_id)
        .await
        .expect("deleting a suspended user should succeed");

    let users = repo_clone.get_all_users().await;
    let user = users
        .iter()
        .find(|u| u.id == user_id)
        .expect("user should still exist (soft delete)");
    assert_eq!(user.status, UserStatus::Deleted);

    // Deleting a suspended user goes through the Suspended -> Deleted edge (a
    // second revoke_all_user_sessions call on an already-empty set), so it
    // must leave no sessions behind either.
    let sessions = repo_clone.get_all_sessions().await;
    assert!(
        sessions.is_empty(),
        "suspend-then-delete must leave no live sessions"
    );
}

#[tokio::test]
async fn suspended_to_suspended_is_a_noop_and_does_not_re_revoke() {
    let (svc, user_id, repo_clone, _sync_clone) = service_with_active_session().await;

    svc.admin_update_user(&operator(), &user_id, &status_patch(UserStatus::Suspended))
        .await
        .expect("suspend should succeed");
    assert!(repo_clone.get_all_sessions().await.is_empty());

    // Plant a sentinel session directly in the repo (bypassing business logic) so we
    // can tell whether the no-op patch below calls `revoke_all_user_sessions` again.
    use oidc_exchange_core::domain::Session;
    use oidc_exchange_core::ports::SessionRepository;
    let sentinel = Session {
        user_id: user_id.clone(),
        refresh_token_hash: oidc_exchange_core::Secret::new("sentinel-hash".to_string()),
        family_id: "fam_0000000000000000000000000d".to_string(),
        generation: 0,
        provider: "mock".to_string(),
        expires_at: chrono::Utc::now() + chrono::Duration::days(1),
        rotated_at: None,
        device_id: None,
        user_agent: None,
        ip_address: None,
        created_at: chrono::Utc::now(),
    };
    repo_clone
        .store_refresh_token(&sentinel)
        .await
        .expect("planting sentinel session should succeed");
    assert_eq!(repo_clone.get_all_sessions().await.len(), 1);

    let updated = svc
        .admin_update_user(&operator(), &user_id, &status_patch(UserStatus::Suspended))
        .await
        .expect("Suspended -> Suspended should be an accepted no-op");
    assert_eq!(updated.status, UserStatus::Suspended);

    let sessions = repo_clone.get_all_sessions().await;
    assert_eq!(
        sessions.len(),
        1,
        "Suspended -> Suspended must not re-trigger revoke_all_user_sessions"
    );
}

#[tokio::test]
async fn deleted_to_active_is_rejected() {
    let (svc, user_id, repo_clone, _sync_clone) = service_with_active_session().await;

    svc.admin_delete_user(&operator(), &user_id)
        .await
        .expect("delete should succeed");

    let result = svc
        .admin_update_user(&operator(), &user_id, &status_patch(UserStatus::Active))
        .await;
    assert!(matches!(result, Err(Error::InvalidRequest { .. })));

    // The rejected patch must not have mutated the user's status.
    let users = repo_clone.get_all_users().await;
    let user = users.iter().find(|u| u.id == user_id).expect("user exists");
    assert_eq!(user.status, UserStatus::Deleted);
}

#[tokio::test]
async fn deleted_to_deleted_is_rejected() {
    let (svc, user_id, repo_clone, _sync_clone) = service_with_active_session().await;

    svc.admin_delete_user(&operator(), &user_id)
        .await
        .expect("delete should succeed");

    let result = svc
        .admin_update_user(&operator(), &user_id, &status_patch(UserStatus::Deleted))
        .await;
    assert!(matches!(result, Err(Error::InvalidRequest { .. })));

    // Deleted admits no status patch at all, including a repeat of itself; the
    // stored version must not have advanced past the original delete's write.
    let users = repo_clone.get_all_users().await;
    let user = users.iter().find(|u| u.id == user_id).expect("user exists");
    assert_eq!(user.status, UserStatus::Deleted);
}

#[tokio::test]
async fn second_delete_on_already_deleted_user_is_rejected() {
    let (svc, user_id, repo_clone, _sync_clone) = service_with_active_session().await;

    svc.admin_delete_user(&operator(), &user_id)
        .await
        .expect("first delete should succeed");
    let version_after_first_delete = repo_clone
        .get_all_users()
        .await
        .into_iter()
        .find(|u| u.id == user_id)
        .expect("user exists")
        .version;

    let result = svc.admin_delete_user(&operator(), &user_id).await;
    assert!(matches!(result, Err(Error::InvalidRequest { .. })));

    // A rejected second delete must not have written to the repository again.
    let version_after_second_delete = repo_clone
        .get_all_users()
        .await
        .into_iter()
        .find(|u| u.id == user_id)
        .expect("user exists")
        .version;
    assert_eq!(version_after_first_delete, version_after_second_delete);
}

#[tokio::test]
async fn admin_update_user_unknown_id_returns_not_found() {
    let repo = MockRepository::new();
    let user_sync = MockUserSync::new();
    let (svc, _repo_clone, _sync_clone) = make_service_with_mocks(repo, user_sync);

    let result = svc
        .admin_update_user(
            &operator(),
            "usr_does_not_exist",
            &status_patch(UserStatus::Suspended),
        )
        .await;
    assert!(matches!(result, Err(Error::NotFound { .. })));

    // Also exercise a non-status patch to confirm the fetch-first check runs
    // regardless of which fields the patch touches.
    let plain_patch = UserPatch {
        email: Some("nobody@example.com".to_string()),
        display_name: None,
        metadata: None,
        claims: None,
        status: None,
    };
    let result2 = svc
        .admin_update_user(&operator(), "usr_does_not_exist", &plain_patch)
        .await;
    assert!(matches!(result2, Err(Error::NotFound { .. })));
}

#[tokio::test]
async fn admin_delete_user_unknown_id_returns_not_found() {
    let repo = MockRepository::new();
    let user_sync = MockUserSync::new();
    let (svc, repo_clone, sync_clone) = make_service_with_mocks(repo, user_sync);

    let result = svc
        .admin_delete_user(&operator(), "usr_does_not_exist")
        .await;
    assert!(matches!(result, Err(Error::NotFound { .. })));

    // The rejected delete must not have written a user row or fired the
    // user-deleted sync notification.
    assert!(repo_clone.get_all_users().await.is_empty());
    assert!(
        sync_clone.calls().await.is_empty(),
        "unknown-id delete must not notify user sync"
    );
}

// ─── Audit event emission for admin mutations ───────────────────────────────

#[tokio::test]
async fn admin_create_user_emits_user_created_audit_event() {
    let repo = MockRepository::new();
    let user_sync = MockUserSync::new();
    let audit = MockAuditLog::new();
    let audit_clone = audit.clone();
    let svc = make_service_with_audit(repo, user_sync, audit, make_config());

    let nu = new_user("audit-create-1", "google");
    let user = svc
        .admin_create_user(&operator(), &nu)
        .await
        .expect("create should succeed");

    let events = audit_clone.events().await;
    assert_eq!(
        events.len(),
        1,
        "admin_create_user should emit exactly one audit event"
    );
    assert_eq!(events[0].event_type, AuditEventType::UserCreated);
    assert_eq!(events[0].actor.as_deref(), Some(user.id.as_str()));
    assert!(
        events[0].ip_address.is_none() && events[0].user_agent.is_none(),
        "admin operations carry no client context, so ip/user_agent must be None"
    );
}

#[tokio::test]
async fn admin_update_user_non_status_patch_emits_user_updated() {
    let repo = MockRepository::new();
    let user_sync = MockUserSync::new();
    let audit = MockAuditLog::new();
    let audit_clone = audit.clone();
    let svc = make_service_with_audit(repo, user_sync, audit, make_config());

    let nu = new_user("audit-update-1", "google");
    let user = svc
        .admin_create_user(&operator(), &nu)
        .await
        .expect("create should succeed");

    let patch = UserPatch {
        email: Some("changed@example.com".to_string()),
        display_name: None,
        metadata: None,
        claims: None,
        status: None,
    };
    svc.admin_update_user(&operator(), &user.id, &patch)
        .await
        .expect("update should succeed");

    let events = audit_clone.events().await;
    // First event is UserCreated from admin_create_user; second is this update.
    assert_eq!(
        events.len(),
        2,
        "create + update should each emit one event"
    );
    assert_eq!(events[1].event_type, AuditEventType::UserUpdated);
    assert_eq!(events[1].actor.as_deref(), Some(user.id.as_str()));
}

#[tokio::test]
async fn admin_update_user_suspend_patch_emits_user_suspended_not_user_updated() {
    let repo = MockRepository::new();
    let user_sync = MockUserSync::new();
    let audit = MockAuditLog::new();
    let audit_clone = audit.clone();
    let svc = make_service_with_audit(repo, user_sync, audit, make_config());

    let nu = new_user("audit-suspend-1", "google");
    let user = svc
        .admin_create_user(&operator(), &nu)
        .await
        .expect("create should succeed");

    svc.admin_update_user(&operator(), &user.id, &status_patch(UserStatus::Suspended))
        .await
        .expect("suspend should succeed");

    let events = audit_clone.events().await;
    assert_eq!(
        events.len(),
        2,
        "create + suspend should each emit one event"
    );
    assert_eq!(events[1].event_type, AuditEventType::UserSuspended);
    assert!(
        !events
            .iter()
            .any(|e| e.event_type == AuditEventType::UserUpdated),
        "a status=Suspended patch must emit UserSuspended, not UserUpdated"
    );
}

#[tokio::test]
async fn admin_delete_user_emits_user_deleted_audit_event() {
    let repo = MockRepository::new();
    let user_sync = MockUserSync::new();
    let audit = MockAuditLog::new();
    let audit_clone = audit.clone();
    let svc = make_service_with_audit(repo, user_sync, audit, make_config());

    let nu = new_user("audit-delete-1", "google");
    let user = svc
        .admin_create_user(&operator(), &nu)
        .await
        .expect("create should succeed");

    svc.admin_delete_user(&operator(), &user.id)
        .await
        .expect("delete should succeed");

    let events = audit_clone.events().await;
    assert_eq!(
        events.len(),
        2,
        "create + delete should each emit one event"
    );
    assert_eq!(events[1].event_type, AuditEventType::UserDeleted);
    assert_eq!(events[1].actor.as_deref(), Some(user.id.as_str()));
}

#[tokio::test]
async fn admin_claims_mutations_emit_user_updated_with_operation_in_detail() {
    let repo = MockRepository::new();
    let user_sync = MockUserSync::new();
    let audit = MockAuditLog::new();
    let audit_clone = audit.clone();
    let svc = make_service_with_audit(repo, user_sync, audit, make_config());

    let nu = new_user("audit-claims-1", "google");
    let user = svc
        .admin_create_user(&operator(), &nu)
        .await
        .expect("create should succeed");

    let mut claims = HashMap::new();
    claims.insert("a".to_string(), json!(1));
    svc.admin_set_claims(&operator(), &user.id, claims.clone())
        .await
        .expect("set claims should succeed");

    let mut merge = HashMap::new();
    merge.insert("b".to_string(), json!(2));
    svc.admin_merge_claims(&operator(), &user.id, merge)
        .await
        .expect("merge claims should succeed");

    svc.admin_clear_claims(&operator(), &user.id)
        .await
        .expect("clear claims should succeed");

    let events = audit_clone.events().await;
    // create + set + merge + clear = 4 events total.
    assert_eq!(
        events.len(),
        4,
        "each claims mutation should emit one event"
    );

    let claims_events = &events[1..];
    let expected_ops = ["set_claims", "merge_claims", "clear_claims"];
    for (event, expected_op) in claims_events.iter().zip(expected_ops.iter()) {
        assert_eq!(event.event_type, AuditEventType::UserUpdated);
        assert_eq!(
            event.detail.get("operation"),
            Some(&json!(*expected_op)),
            "claims mutation event should record its operation name in detail"
        );
    }
}

// ─── Negative space: read-only admin operations emit nothing ───────────────

#[tokio::test]
async fn admin_reads_emit_no_audit_events() {
    let repo = MockRepository::new();
    let user_sync = MockUserSync::new();
    let audit = MockAuditLog::new();
    let audit_clone = audit.clone();

    // Pre-populate a user directly through the repo, bypassing the service so
    // no audit event is emitted by this setup step.
    let nu = new_user("audit-read-1", "google");
    repo.create_user(&nu)
        .await
        .expect("pre-create user should succeed");
    let user_id = repo.get_all_users().await[0].id.clone();

    let svc = make_service_with_audit(repo, user_sync, audit, make_config());

    svc.admin_get_user(&user_id)
        .await
        .expect("get_user should succeed");
    svc.admin_list_users(None, Some(10))
        .await
        .expect("list_users should succeed");
    svc.admin_stats().await.expect("stats should succeed");
    svc.admin_get_claims(&user_id)
        .await
        .expect("get_claims should succeed");

    let events = audit_clone.events().await;
    assert!(
        events.is_empty(),
        "read-only admin operations must never emit audit events, got: {:?}",
        events
    );
}

// ─── Mandatory audit durability for admin mutations ────────────────────────

/// Admin mutation auditing bypasses threshold filtering and fails closed when
/// its mandatory audit sink is configured to enforce durability.
#[tokio::test]
async fn admin_create_user_enforced_audit_failure_propagates_durability_error_and_skips_sync() {
    let config = Config::resolve(RawConfig {
        audit: oidc_exchange_core::config::RawAuditConfig {
            adapter: "noop".to_string(),
            durability: "enforce".to_string(),
            // This intentionally excludes Notice from threshold-filtered
            // best-effort emission; admin auditing must still be attempted.
            emit_threshold: "warning".to_string(),
            blocking_threshold: "emergency".to_string(),
            sqs: None,
        },
        ..base_raw_config()
    })
    .expect("test config should resolve");

    let repo = MockRepository::new();
    let user_sync = MockUserSync::new();
    let sync_clone = user_sync.clone();
    let audit = MockAuditLog::new();
    audit.set_fail_mode(true).await;
    let svc = make_service_with_audit(repo, user_sync, audit, config);

    let nu = new_user("audit-blocking-1", "google");
    let err = svc
        .admin_create_user(&operator(), &nu)
        .await
        .expect_err("an enforced mandatory audit failure must propagate as Err");

    match err {
        Error::SecurityAuditDurability { .. } => {}
        other => panic!("expected SecurityAuditDurability, got: {:?}", other),
    }

    assert!(
        sync_clone.calls().await.is_empty(),
        "a mandatory audit failure must short-circuit before the best-effort sync notify runs"
    );
}

// ─── Operator attribution on admin mutations (task 05) ─────────────────────

/// The principal recorded on a successful mutation's audit event must be the
/// acting operator, carried verbatim — that is the whole point of the
/// attribution field.
#[tokio::test]
async fn admin_mutations_record_the_acting_operator() {
    let repo = MockRepository::new();
    let user_sync = MockUserSync::new();
    let audit = MockAuditLog::new();
    let audit_clone = audit.clone();
    let svc = make_service_with_audit(repo, user_sync, audit, make_config());

    let nu = new_user("attribution-1", "google");
    let user = svc
        .admin_create_user(&operator(), &nu)
        .await
        .expect("create should succeed");

    svc.admin_update_user(&operator(), &user.id, &status_patch(UserStatus::Suspended))
        .await
        .expect("update should succeed");
    svc.admin_set_claims(&operator(), &user.id, HashMap::new())
        .await
        .expect("set_claims should succeed");
    svc.admin_merge_claims(&operator(), &user.id, HashMap::new())
        .await
        .expect("merge_claims should succeed");
    svc.admin_clear_claims(&operator(), &user.id)
        .await
        .expect("clear_claims should succeed");
    svc.admin_delete_user(&operator(), &user.id)
        .await
        .expect("delete should succeed");

    let events = audit_clone.events().await;
    assert_eq!(
        events.len(),
        6,
        "each of the six mutations emits exactly one event"
    );
    // Every emitted event must carry the acting principal, and the actor is
    // the subject of the action while the operator is who performed it — the
    // two fields answer different questions on the same record.
    for event in &events {
        assert_eq!(event.actor.as_deref(), Some(user.id.as_str()));
        assert_eq!(
            event.operator.as_ref(),
            Some(&operator()),
            "every admin mutation must carry the acting principal"
        );
    }
    let types: Vec<AuditEventType> = events.iter().map(|e| e.event_type.clone()).collect();
    assert_eq!(
        types,
        vec![
            AuditEventType::UserCreated,
            AuditEventType::UserSuspended,
            AuditEventType::UserUpdated,
            AuditEventType::UserUpdated,
            AuditEventType::UserUpdated,
            AuditEventType::UserDeleted,
        ],
        "attribution must not disturb the per-operation event classification"
    );
}

/// The shared-secret compatibility path records the explicit `unattributed`
/// principal rather than omitting identity: an audit reader can distinguish
/// "authenticated as nobody" from "field missing" without configuration.
#[tokio::test]
async fn shared_secret_mutations_record_the_unattributed_principal() {
    let repo = MockRepository::new();
    let user_sync = MockUserSync::new();
    let audit = MockAuditLog::new();
    let audit_clone = audit.clone();
    let svc = make_service_with_audit(repo, user_sync, audit, make_config());

    let nu = new_user("attribution-2", "google");
    let user = svc
        .admin_create_user(&OperatorPrincipal::unattributed(), &nu)
        .await
        .expect("create should succeed");

    let events = audit_clone.events().await;
    assert_eq!(events.len(), 1, "create emits exactly one event");
    let operator = events[0]
        .operator
        .as_ref()
        .expect("the shared-secret path must still attribute explicitly");
    assert_eq!(operator.id, "unattributed");
    assert_eq!(
        operator.mechanism,
        OperatorAuthMechanism::SharedSecret,
        "the unattributed id travels with its mechanism so the record is self-describing"
    );
    assert_eq!(
        events[0].actor.as_deref(),
        Some(user.id.as_str()),
        "actor (subject) and operator (performer) remain distinct fields"
    );
}

/// Exchange-plane events keep null operator attribution: `create_audit_event`
/// leaves `operator` unset, and nothing outside `attributed()` stamps it.
#[test]
fn exchange_plane_events_retain_null_operator_attribution() {
    let event = oidc_exchange_core::service::create_audit_event(
        AuditEventType::TokenExchange,
        oidc_exchange_core::domain::AuditSeverity::Info,
        oidc_exchange_core::domain::AuditOutcome::Success,
        Some("usr_subject".to_string()),
        Some("google".to_string()),
        oidc_exchange_core::domain::ClientAddr::Unknown,
        None,
    );

    assert!(
        event.operator.is_none(),
        "an exchange-plane event has no operator; the field stays None, never a placeholder"
    );
    assert_eq!(event.actor.as_deref(), Some("usr_subject"));
}

// ─── Bounded cursor pagination (task 08): core clamp + traversal ───────────

use oidc_exchange_core::domain::{UserPage, DEFAULT_ADMIN_PAGE_SIZE, MAX_ADMIN_PAGE_SIZE};
use std::sync::atomic::{AtomicU32, Ordering as AtomicOrdering};
use std::sync::Arc;

/// A `UserRepository` decorator that records the `limit` and `cursor` the
/// service actually handed to the port, so tests can prove the clamp happens
/// in the core — before any adapter is reached — rather than in an HTTP
/// handler that a non-HTTP caller could bypass.
#[derive(Clone)]
struct RecordingLimitRepository {
    inner: MockRepository,
    last_limit: Arc<AtomicU32>,
    call_count: Arc<AtomicU32>,
}

impl RecordingLimitRepository {
    fn new(inner: MockRepository) -> Self {
        Self {
            inner,
            last_limit: Arc::new(AtomicU32::new(0)),
            call_count: Arc::new(AtomicU32::new(0)),
        }
    }

    fn last_limit(&self) -> u32 {
        self.last_limit.load(AtomicOrdering::SeqCst)
    }

    fn calls(&self) -> u32 {
        self.call_count.load(AtomicOrdering::SeqCst)
    }
}

#[async_trait::async_trait]
impl UserRepository for RecordingLimitRepository {
    async fn get_user_by_id(
        &self,
        user_id: &str,
    ) -> oidc_exchange_core::error::Result<Option<oidc_exchange_core::domain::User>> {
        self.inner.get_user_by_id(user_id).await
    }

    async fn get_user_by_external_id(
        &self,
        external_id: &str,
        provider: &str,
    ) -> oidc_exchange_core::error::Result<Option<oidc_exchange_core::domain::User>> {
        self.inner
            .get_user_by_external_id(external_id, provider)
            .await
    }

    async fn create_user(
        &self,
        user: &NewUser,
    ) -> oidc_exchange_core::error::Result<oidc_exchange_core::domain::User> {
        self.inner.create_user(user).await
    }

    async fn update_user(
        &self,
        user_id: &str,
        patch: &UserPatch,
    ) -> oidc_exchange_core::error::Result<oidc_exchange_core::domain::User> {
        self.inner.update_user(user_id, patch).await
    }

    async fn delete_user(&self, user_id: &str) -> oidc_exchange_core::error::Result<()> {
        self.inner.delete_user(user_id).await
    }

    async fn count_by_status(&self) -> oidc_exchange_core::error::Result<HashMap<String, u64>> {
        self.inner.count_by_status().await
    }

    async fn list_users(
        &self,
        cursor: Option<&str>,
        limit: u32,
    ) -> oidc_exchange_core::error::Result<UserPage> {
        self.call_count.fetch_add(1, AtomicOrdering::SeqCst);
        self.last_limit.store(limit, AtomicOrdering::SeqCst);
        assert!(
            cursor.is_none(),
            "this recording mock only asserts limit clamping; cursor is None in these tests"
        );
        self.inner.list_users(cursor, limit).await
    }
}

fn make_service_with_repo(repo: MockRepository) -> AppService {
    make_service_with_mocks(repo, MockUserSync::new()).0
}

fn make_service_recording(repo: RecordingLimitRepository) -> AppService {
    let provider = MockIdentityProvider::new("mock");
    let provider_id = provider.provider_id().to_string();
    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    providers.insert(provider_id, Box::new(provider));

    AppService::new(
        Box::new(repo),
        Box::new(MockRepository::new()),
        Box::new(MockKeyManager::new()),
        Box::new(MockAuditLog::new()),
        Box::new(MockUserSync::new()),
        Box::new(MockRateLimiter::new()),
        providers,
        make_config(),
    )
}

/// An above-bound `limit` is clamped to `MAX_ADMIN_PAGE_SIZE` in the core,
/// before the port is reached: the adapter observes exactly 200, never the
/// caller's 500.
#[tokio::test]
async fn admin_list_users_clamps_above_bound_limit_before_the_adapter() {
    let recording = RecordingLimitRepository::new(MockRepository::new());
    let svc = make_service_recording(recording.clone());

    let page = svc
        .admin_list_users(None, Some(MAX_ADMIN_PAGE_SIZE + 300))
        .await
        .expect("an above-bound limit clamps rather than errors");

    assert!(page.next_cursor.is_none(), "an empty store is exhausted");
    assert_eq!(recording.calls(), 1, "exactly one adapter call");
    assert_eq!(
        recording.last_limit(),
        MAX_ADMIN_PAGE_SIZE,
        "the adapter must observe the clamped limit, not the caller's"
    );
}

/// An absent `limit` resolves to the documented default (50) inside the core.
#[tokio::test]
async fn admin_list_users_resolves_the_default_limit_in_the_core() {
    let recording = RecordingLimitRepository::new(MockRepository::new());
    let svc = make_service_recording(recording.clone());

    svc.admin_list_users(None, None)
        .await
        .expect("a default listing succeeds");

    assert_eq!(
        recording.last_limit(),
        DEFAULT_ADMIN_PAGE_SIZE,
        "None must resolve to DEFAULT_ADMIN_PAGE_SIZE before the adapter"
    );
}

/// `limit = 0` violates the published schema's `minimum: 1` and is rejected
/// with `InvalidRequest` *without* reaching the adapter at all.
#[tokio::test]
async fn admin_list_users_rejects_zero_limit_without_touching_the_adapter() {
    let recording = RecordingLimitRepository::new(MockRepository::new());
    let svc = make_service_recording(recording.clone());

    let err = svc
        .admin_list_users(None, Some(0))
        .await
        .expect_err("limit 0 is below the documented minimum");

    match err {
        Error::InvalidRequest { reason } => {
            assert!(
                reason.contains("at least 1"),
                "the rejection must name the minimum, got: {reason}"
            );
        }
        other => panic!("expected Error::InvalidRequest, got {other:?}"),
    }
    assert_eq!(
        recording.calls(),
        0,
        "a rejected limit must never reach the adapter"
    );
}

/// A full traversal over the mock's keyset ordering: pages of 2 over 7 users
/// cover every row exactly once and terminate only at a null cursor.
#[tokio::test]
async fn admin_list_users_traverses_every_user_exactly_once_until_null_cursor() {
    let repo = MockRepository::new();
    let svc = make_service_with_repo(repo);

    let total = 7;
    for i in 0..total {
        svc.admin_create_user(&operator(), &new_user(&format!("page-{i}"), "google"))
            .await
            .expect("seed user");
    }

    let mut seen: Vec<String> = Vec::new();
    let mut cursor: Option<String> = None;
    let mut pages = 0;
    loop {
        let page = svc
            .admin_list_users(cursor.as_deref(), Some(2))
            .await
            .expect("each page succeeds");
        pages += 1;
        seen.extend(page.users.iter().map(|u| u.id.clone()));
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
        assert!(pages < 100, "traversal must terminate");
    }

    assert_eq!(seen.len(), total, "every user is returned exactly once");
    assert_eq!(
        seen.iter().collect::<std::collections::HashSet<_>>().len(),
        total,
        "no duplicates across adjacent pages"
    );
    assert_eq!(pages, 4, "7 users at limit 2 = pages of 2+2+2+1");
}

/// A tampered cursor is a caller fault: `InvalidRequest`, deterministically.
#[tokio::test]
async fn admin_list_users_rejects_a_tampered_cursor_as_invalid_request() {
    let repo = MockRepository::new();
    let svc = make_service_with_repo(repo);

    for bad_cursor in ["garbage", "aGVsbG8=", ""] {
        let err = svc
            .admin_list_users(Some(bad_cursor), Some(10))
            .await
            .expect_err("a tampered cursor must be rejected");
        match err {
            Error::InvalidRequest { .. } => {}
            other => panic!("expected InvalidRequest for {bad_cursor:?}, got {other:?}"),
        }
    }
}
