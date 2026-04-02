use std::collections::HashMap;

use serde_json::json;

use oidc_exchange_core::config::{AppConfig, ServerConfig, TokenConfig};
use oidc_exchange_core::domain::{NewUser, UserPatch, UserStatus};
use oidc_exchange_core::ports::IdentityProvider;
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
    assert!(claims.get("a").is_none());
    assert!(claims.get("b").is_none());
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
