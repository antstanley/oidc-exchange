use std::collections::HashMap;

use serde_json::json;

use oidc_exchange_core::config::{AppConfig, AuditConfig, ServerConfig, TokenConfig};
use oidc_exchange_core::domain::{AuditEventType, NewUser, UserPatch, UserStatus};
use oidc_exchange_core::error::Error;
use oidc_exchange_core::ports::{IdentityProvider, UserRepository};
use oidc_exchange_core::service::exchange::ExchangeRequest;
use oidc_exchange_core::service::AppService;

use oidc_exchange_test_utils::{
    MockAuditLog, MockIdentityProvider, MockKeyManager, MockRepository, MockUserSync, UserSyncCall,
};

fn make_config() -> AppConfig {
    AppConfig {
        server: ServerConfig {
            issuer: "https://auth.test.com".to_string(),
            ..Default::default()
        },
        token: TokenConfig {
            access_token_ttl: "15m".to_string(),
            refresh_token_ttl: "30d".to_string(),
            audience: Some("https://api.test.com".to_string()),
            ..Default::default()
        },
        ..Default::default()
    }
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
    config: AppConfig,
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
        .admin_create_user(&nu)
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
        .admin_create_user(&nu)
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
        .admin_update_user(&user.id, &patch)
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
        .admin_create_user(&nu)
        .await
        .expect("create should succeed");

    // Set initial claims {"a": 1}
    let mut initial = HashMap::new();
    initial.insert("a".to_string(), json!(1));
    svc.admin_set_claims(&user.id, initial)
        .await
        .expect("set claims should succeed");

    // Merge {"b": 2}
    let mut merge = HashMap::new();
    merge.insert("b".to_string(), json!(2));
    svc.admin_merge_claims(&user.id, merge)
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
        .admin_create_user(&nu)
        .await
        .expect("create should succeed");

    // Set initial claims {"a": 1, "b": 2}
    let mut initial = HashMap::new();
    initial.insert("a".to_string(), json!(1));
    initial.insert("b".to_string(), json!(2));
    svc.admin_set_claims(&user.id, initial)
        .await
        .expect("set claims should succeed");

    // Replace with {"c": 3}
    let mut replacement = HashMap::new();
    replacement.insert("c".to_string(), json!(3));
    svc.admin_set_claims(&user.id, replacement)
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
        .admin_create_user(&nu)
        .await
        .expect("create should succeed");

    // Set some claims
    let mut initial = HashMap::new();
    initial.insert("x".to_string(), json!("hello"));
    initial.insert("y".to_string(), json!(42));
    svc.admin_set_claims(&user.id, initial)
        .await
        .expect("set claims should succeed");

    // Clear
    svc.admin_clear_claims(&user.id)
        .await
        .expect("clear claims should succeed");

    // Verify empty
    let claims = svc
        .admin_get_claims(&user.id)
        .await
        .expect("get claims should succeed");
    assert!(claims.is_empty(), "claims should be empty after clear");
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
    let result = svc.admin_set_claims("usr_does_not_exist", claims).await;
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
    let result = svc.admin_merge_claims("usr_does_not_exist", claims).await;
    assert!(matches!(result, Err(Error::NotFound { .. })));

    // The rejected merge must not have written a user row.
    assert!(repo_clone.get_all_users().await.is_empty());
}

#[tokio::test]
async fn admin_clear_claims_unknown_id_returns_not_found() {
    let repo = MockRepository::new();
    let user_sync = MockUserSync::new();
    let (svc, repo_clone, _sync_clone) = make_service_with_mocks(repo, user_sync);

    let result = svc.admin_clear_claims("usr_does_not_exist").await;
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
        code: Some("auth-code".to_string()),
        redirect_uri: Some("https://app.test.com/callback".to_string()),
        id_token: None,
        provider: "mock".to_string(),
        ..Default::default()
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
    svc.admin_delete_user(&user_id)
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
        code: Some("auth-code".to_string()),
        redirect_uri: Some("https://app.test.com/callback".to_string()),
        id_token: None,
        provider: "mock".to_string(),
        ..Default::default()
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
        .admin_update_user(&user_id, &status_patch(UserStatus::Suspended))
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
        .admin_update_user(&user_id, &status_patch(UserStatus::Deleted))
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

    svc.admin_update_user(&user_id, &status_patch(UserStatus::Suspended))
        .await
        .expect("suspend should succeed");
    assert!(repo_clone.get_all_sessions().await.is_empty());

    let reactivated = svc
        .admin_update_user(&user_id, &status_patch(UserStatus::Active))
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

    svc.admin_update_user(&user_id, &status_patch(UserStatus::Suspended))
        .await
        .expect("suspend should succeed");

    svc.admin_delete_user(&user_id)
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

    svc.admin_update_user(&user_id, &status_patch(UserStatus::Suspended))
        .await
        .expect("suspend should succeed");
    assert!(repo_clone.get_all_sessions().await.is_empty());

    // Plant a sentinel session directly in the repo (bypassing business logic) so we
    // can tell whether the no-op patch below calls `revoke_all_user_sessions` again.
    use oidc_exchange_core::domain::Session;
    use oidc_exchange_core::ports::SessionRepository;
    let sentinel = Session {
        user_id: user_id.clone(),
        refresh_token_hash: "sentinel-hash".to_string(),
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
        .admin_update_user(&user_id, &status_patch(UserStatus::Suspended))
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

    svc.admin_delete_user(&user_id)
        .await
        .expect("delete should succeed");

    let result = svc
        .admin_update_user(&user_id, &status_patch(UserStatus::Active))
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

    svc.admin_delete_user(&user_id)
        .await
        .expect("delete should succeed");

    let result = svc
        .admin_update_user(&user_id, &status_patch(UserStatus::Deleted))
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

    svc.admin_delete_user(&user_id)
        .await
        .expect("first delete should succeed");
    let version_after_first_delete = repo_clone
        .get_all_users()
        .await
        .into_iter()
        .find(|u| u.id == user_id)
        .expect("user exists")
        .version;

    let result = svc.admin_delete_user(&user_id).await;
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
        .admin_update_user("usr_does_not_exist", &status_patch(UserStatus::Suspended))
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
        .admin_update_user("usr_does_not_exist", &plain_patch)
        .await;
    assert!(matches!(result2, Err(Error::NotFound { .. })));
}

#[tokio::test]
async fn admin_delete_user_unknown_id_returns_not_found() {
    let repo = MockRepository::new();
    let user_sync = MockUserSync::new();
    let (svc, repo_clone, sync_clone) = make_service_with_mocks(repo, user_sync);

    let result = svc.admin_delete_user("usr_does_not_exist").await;
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
        .admin_create_user(&nu)
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
        .admin_create_user(&nu)
        .await
        .expect("create should succeed");

    let patch = UserPatch {
        email: Some("changed@example.com".to_string()),
        display_name: None,
        metadata: None,
        claims: None,
        status: None,
    };
    svc.admin_update_user(&user.id, &patch)
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
        .admin_create_user(&nu)
        .await
        .expect("create should succeed");

    svc.admin_update_user(&user.id, &status_patch(UserStatus::Suspended))
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
        .admin_create_user(&nu)
        .await
        .expect("create should succeed");

    svc.admin_delete_user(&user.id)
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
        .admin_create_user(&nu)
        .await
        .expect("create should succeed");

    let mut claims = HashMap::new();
    claims.insert("a".to_string(), json!(1));
    svc.admin_set_claims(&user.id, claims.clone())
        .await
        .expect("set claims should succeed");

    let mut merge = HashMap::new();
    merge.insert("b".to_string(), json!(2));
    svc.admin_merge_claims(&user.id, merge)
        .await
        .expect("merge claims should succeed");

    svc.admin_clear_claims(&user.id)
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
    svc.admin_list_users(0, 10)
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

// ─── Blocking audit rules distinguish admin audit from best-effort sync ────

/// A blocking-threshold audit failure on an admin mutation propagates as
/// `Err`, and — because the emission happens before the best-effort
/// user-sync notify — the notify call never fires. This is the contrast
/// with `notify_user_created`/`updated`/`deleted`, which only warn-and-log on
/// failure and never abort the operation.
#[tokio::test]
async fn admin_create_user_blocking_audit_failure_propagates_err_and_skips_sync() {
    // `blocking_threshold: "info"` covers every severity from Emergency down
    // to Info, so the Notice-severity `UserCreated` emission is blocking.
    let config = AppConfig {
        audit: AuditConfig {
            blocking_threshold: "info".to_string(),
            ..Default::default()
        },
        ..make_config()
    };

    let repo = MockRepository::new();
    let user_sync = MockUserSync::new();
    let sync_clone = user_sync.clone();
    let audit = MockAuditLog::new();
    audit.set_fail_mode(true).await;
    let svc = make_service_with_audit(repo, user_sync, audit, config);

    let nu = new_user("audit-blocking-1", "google");
    let err = svc
        .admin_create_user(&nu)
        .await
        .expect_err("a blocking audit failure must propagate as Err");

    match err {
        Error::AuditError { .. } => {}
        other => panic!("expected AuditError to propagate, got: {:?}", other),
    }

    assert!(
        sync_clone.calls().await.is_empty(),
        "a blocking audit failure must short-circuit before the best-effort sync notify runs"
    );
}
