use oidc_exchange_core::domain::Session;
use oidc_exchange_core::ports::SessionRepository;
use oidc_exchange_core::Secret;

use oidc_exchange_test_utils::MockRepository;

#[tokio::test]
async fn cleanup_expired_sessions_removes_stale_entries() {
    let repo = MockRepository::new();

    let now = chrono::Utc::now();

    // Store an expired session
    let expired_session = Session {
        user_id: "usr_1".to_string(),
        refresh_token_hash: Secret::new("hash_expired".to_string()),
        family_id: "fam_0000000000000000000000000a".to_string(),
        generation: 0,
        provider: "mock".to_string(),
        expires_at: now - chrono::Duration::hours(1),
        rotated_at: None,
        device_id: None,
        user_agent: None,
        ip_address: None,
        created_at: now - chrono::Duration::hours(25),
    };
    repo.store_refresh_token(&expired_session).await.unwrap();

    // Store an active session
    let active_session = Session {
        user_id: "usr_2".to_string(),
        refresh_token_hash: Secret::new("hash_active".to_string()),
        family_id: "fam_0000000000000000000000000b".to_string(),
        generation: 0,
        provider: "mock".to_string(),
        expires_at: now + chrono::Duration::hours(24),
        rotated_at: None,
        device_id: None,
        user_agent: None,
        ip_address: None,
        created_at: now,
    };
    repo.store_refresh_token(&active_session).await.unwrap();

    assert_eq!(repo.get_all_sessions().await.len(), 2);

    // Cleanup
    let deleted = repo.cleanup_expired_sessions().await.unwrap();
    assert_eq!(deleted, 1, "should delete 1 expired session");

    let remaining = repo.get_all_sessions().await;
    assert_eq!(remaining.len(), 1, "should have 1 active session left");
    assert!(
        remaining[0].refresh_token_hash == Secret::new("hash_active".to_string()),
        "constant-time equality must hold across a round trip"
    );
}

#[tokio::test]
async fn cleanup_no_expired_sessions_returns_zero() {
    let repo = MockRepository::new();
    let now = chrono::Utc::now();

    let active_session = Session {
        user_id: "usr_1".to_string(),
        refresh_token_hash: Secret::new("hash_active".to_string()),
        family_id: "fam_0000000000000000000000000c".to_string(),
        generation: 0,
        provider: "mock".to_string(),
        expires_at: now + chrono::Duration::hours(24),
        rotated_at: None,
        device_id: None,
        user_agent: None,
        ip_address: None,
        created_at: now,
    };
    repo.store_refresh_token(&active_session).await.unwrap();

    let deleted = repo.cleanup_expired_sessions().await.unwrap();
    assert_eq!(deleted, 0, "should delete 0 sessions when none expired");
    assert_eq!(repo.get_all_sessions().await.len(), 1);
}
