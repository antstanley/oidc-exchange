use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use sha2::{Digest, Sha256};

use oidc_exchange_core::config::{
    Config, RawAuditConfig, RawConfig, RawRegistrationConfig, RawServerConfig, RawTelemetryConfig,
    RawTokenConfig,
};
use oidc_exchange_core::domain::{
    is_valid_family_id, AccessTokenClaims, AuditEventType, AuditFailure, AuditOutcome,
    AuditSeverity, IdentityClaims, NewUser, User, UserPatch, UserStatus,
};
use oidc_exchange_core::error::{Error, Result};
use oidc_exchange_core::ports::{IdentityProvider, UserRepository};
use oidc_exchange_core::service::exchange::{ExchangeCredential, ExchangeRequest};
use oidc_exchange_core::service::AppService;

use oidc_exchange_test_utils::{
    MockAuditLog, MockIdentityProvider, MockKeyManager, MockRepository, MockUserSync, UserSyncCall,
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

/// Deterministically unique `jti` for test-built assertions, so two exchanges
/// never share a replay marker unless a test deliberately reuses one.
fn unique_jti() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    format!("jti-{}", COUNTER.fetch_add(1, Ordering::SeqCst))
}

/// Raw claims a real validator would return and every binding control accepts:
/// an `exp` inside the default lifetime ceiling and a fresh `jti`.
fn verified_raw_claims() -> HashMap<String, serde_json::Value> {
    let mut raw = HashMap::new();
    raw.insert(
        "exp".to_string(),
        serde_json::json!(chrono::Utc::now().timestamp() + 600),
    );
    raw.insert("jti".to_string(), serde_json::json!(unique_jti()));
    raw
}

fn make_config() -> Config {
    Config::resolve(base_raw_config()).expect("test config should resolve")
}

fn make_service(repo: MockRepository, provider: MockIdentityProvider) -> AppService {
    make_service_with_config(repo, provider, make_config())
}

fn make_service_with_config(
    repo: MockRepository,
    provider: MockIdentityProvider,
    config: Config,
) -> AppService {
    let provider_id = provider.provider_id().to_string();
    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    providers.insert(provider_id, Box::new(provider));

    AppService::new(
        Box::new(repo.clone()),
        Box::new(repo),
        Box::new(MockKeyManager::new()),
        Box::new(MockAuditLog::new()),
        Box::new(MockUserSync::new()),
        Box::new(oidc_exchange_test_utils::MockRateLimiter::new()),
        providers,
        config,
    )
}

/// Build a service whose `UserRepository` is a caller-supplied decorator
/// (used to model a JIT-registration racer deterministically) while sessions
/// still land in the shared `MockRepository`.
fn make_service_with_user_repo(
    user_repo: Box<dyn UserRepository>,
    session_repo: MockRepository,
    provider: MockIdentityProvider,
    config: Config,
) -> AppService {
    make_service_with_user_repo_and_audit(
        user_repo,
        session_repo,
        provider,
        config,
        MockAuditLog::new(),
    )
}

fn make_service_with_user_repo_and_audit(
    user_repo: Box<dyn UserRepository>,
    session_repo: MockRepository,
    provider: MockIdentityProvider,
    config: Config,
    audit: MockAuditLog,
) -> AppService {
    let provider_id = provider.provider_id().to_string();
    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    providers.insert(provider_id, Box::new(provider));

    AppService::new(
        user_repo,
        Box::new(session_repo),
        Box::new(MockKeyManager::new()),
        Box::new(audit),
        Box::new(MockUserSync::new()),
        Box::new(oidc_exchange_test_utils::MockRateLimiter::new()),
        providers,
        config,
    )
}

/// Builds a service whose `UserSync` is a caller-supplied `MockUserSync`, so
/// the test can inspect (or fail-mode) sync notifications after the exchange
/// runs.
fn make_service_with_user_sync(
    repo: MockRepository,
    provider: MockIdentityProvider,
    config: Config,
    user_sync: MockUserSync,
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
        Box::new(oidc_exchange_test_utils::MockRateLimiter::new()),
        providers,
        config,
    )
}

/// Builds a service whose `AuditLog` is a caller-supplied `MockAuditLog`, so
/// the test can inspect recorded events after the exchange runs.
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
        Box::new(oidc_exchange_test_utils::MockRateLimiter::new()),
        providers,
        config,
    )
}

/// Decorates a `UserRepository` so `get_user_by_external_id` reports "not
/// found" for a fixed number of calls before delegating to the inner
/// repository. Models the read side of a JIT-registration race
/// deterministically: this racer's lookup ran before a concurrent winner's
/// write committed, so its subsequent `create_user` call (which always
/// delegates to the real, shared `MockRepository`) observes the winner's row
/// and conflicts — without relying on actual thread scheduling.
struct StaleReadUserRepository {
    inner: MockRepository,
    stale_reads_remaining: AtomicU32,
}

impl StaleReadUserRepository {
    fn new(inner: MockRepository, stale_reads: u32) -> Self {
        Self {
            inner,
            stale_reads_remaining: AtomicU32::new(stale_reads),
        }
    }
}

#[async_trait::async_trait]
impl UserRepository for StaleReadUserRepository {
    async fn get_user_by_id(&self, user_id: &str) -> Result<Option<User>> {
        self.inner.get_user_by_id(user_id).await
    }

    async fn get_user_by_external_id(
        &self,
        external_id: &str,
        provider: &str,
    ) -> Result<Option<User>> {
        // Assertion: the counter never underflows past zero — `fetch_sub` is
        // only called while `remaining > 0`.
        let remaining = self.stale_reads_remaining.load(Ordering::SeqCst);
        if remaining > 0 {
            self.stale_reads_remaining.fetch_sub(1, Ordering::SeqCst);
            return Ok(None);
        }
        self.inner
            .get_user_by_external_id(external_id, provider)
            .await
    }

    async fn create_user(&self, user: &NewUser) -> Result<User> {
        self.inner.create_user(user).await
    }

    async fn update_user(&self, user_id: &str, patch: &UserPatch) -> Result<User> {
        self.inner.update_user(user_id, patch).await
    }

    async fn delete_user(&self, user_id: &str) -> Result<()> {
        self.inner.delete_user(user_id).await
    }

    async fn count_by_status(&self) -> Result<HashMap<String, u64>> {
        self.inner.count_by_status().await
    }

    async fn list_users(&self, offset: u64, limit: u64) -> Result<Vec<User>> {
        self.inner.list_users(offset, limit).await
    }
}

/// Decorates a `UserRepository` whose `create_user` always fails with a
/// fixed, non-`Conflict` error, and counts `get_user_by_external_id` calls.
/// Used to assert a non-`Conflict` `create_user` error propagates without a
/// silent re-lookup.
struct FailingCreateUserRepository {
    inner: MockRepository,
    lookup_calls: std::sync::Arc<AtomicU32>,
}

impl FailingCreateUserRepository {
    /// Returns the decorator plus a shared handle onto its lookup-call
    /// counter, so a test can inspect the count after the decorator itself
    /// has been moved into a `Box<dyn UserRepository>`.
    fn new(inner: MockRepository) -> (Self, std::sync::Arc<AtomicU32>) {
        let lookup_calls = std::sync::Arc::new(AtomicU32::new(0));
        (
            Self {
                inner,
                lookup_calls: lookup_calls.clone(),
            },
            lookup_calls,
        )
    }
}

#[async_trait::async_trait]
impl UserRepository for FailingCreateUserRepository {
    async fn get_user_by_id(&self, user_id: &str) -> Result<Option<User>> {
        self.inner.get_user_by_id(user_id).await
    }

    async fn get_user_by_external_id(
        &self,
        external_id: &str,
        provider: &str,
    ) -> Result<Option<User>> {
        self.lookup_calls.fetch_add(1, Ordering::SeqCst);
        self.inner
            .get_user_by_external_id(external_id, provider)
            .await
    }

    async fn create_user(&self, _user: &NewUser) -> Result<User> {
        Err(Error::StoreError {
            detail: "simulated infrastructure failure".to_string(),
        })
    }

    async fn update_user(&self, user_id: &str, patch: &UserPatch) -> Result<User> {
        self.inner.update_user(user_id, patch).await
    }

    async fn delete_user(&self, user_id: &str) -> Result<()> {
        self.inner.delete_user(user_id).await
    }

    async fn count_by_status(&self) -> Result<HashMap<String, u64>> {
        self.inner.count_by_status().await
    }

    async fn list_users(&self, offset: u64, limit: u64) -> Result<Vec<User>> {
        self.inner.list_users(offset, limit).await
    }
}

#[tokio::test]
async fn exchange_happy_path_creates_user_and_returns_tokens() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let svc = make_service(repo.clone(), provider);

    let request = ExchangeRequest {
        provider_access_token: None,
        credential: ExchangeCredential::AuthorizationCode {
            code: "auth-code-123".to_string(),
            redirect_uri: "https://app.test.com/callback".to_string(),
        },
        provider: "mock".to_string(),
        ip_address: None,
        user_agent: None,
        device_id: None,
    };

    let response = svc
        .exchange(request)
        .await
        .expect("exchange should succeed");

    // Should return a Bearer token response
    assert_eq!(response.token_type, "Bearer");
    assert_eq!(response.expires_in, 900); // 15m = 900s
    assert!(!response.access_token.is_empty());
    assert!(response.refresh_token.is_some());

    // Access token should be a valid JWT structure (3 dot-separated parts)
    let parts: Vec<&str> = response.access_token.split('.').collect();
    assert_eq!(parts.len(), 3, "JWT should have 3 parts");

    // Decode and verify the header: the RFC 9068 access-token media type is
    // what later lets a validator tell this artifact apart from any other
    // JWT the same key signs.
    let header_bytes = URL_SAFE_NO_PAD
        .decode(parts[0])
        .expect("header should be valid base64url");
    let header: serde_json::Value =
        serde_json::from_slice(&header_bytes).expect("header should deserialize");
    assert_eq!(
        header["typ"], "at+jwt",
        "access tokens must be minted as at+jwt"
    );
    assert_eq!(header["alg"], "EdDSA");
    assert_eq!(header["kid"], "test-key-1");

    // Decode and verify the payload claims
    let payload_bytes = URL_SAFE_NO_PAD
        .decode(parts[1])
        .expect("payload should be valid base64url");
    let claims: AccessTokenClaims =
        serde_json::from_slice(&payload_bytes).expect("payload should deserialize");
    assert_eq!(claims.iss, "https://auth.test.com");
    assert_eq!(claims.aud, "https://api.test.com");
    assert!(claims.sub.starts_with("usr_"));

    // A new user should have been created
    let users = repo.get_all_users().await;
    assert_eq!(users.len(), 1);
    assert_eq!(users[0].external_id, "test-subject");
    assert_eq!(users[0].email.as_deref(), Some("test@example.com"));
    assert_eq!(users[0].provider, "mock");

    // A session should have been stored with the hashed refresh token
    let sessions = repo.get_all_sessions().await;
    assert_eq!(sessions.len(), 1);

    let refresh_token = response.refresh_token.unwrap();
    let expected_hash = hex::encode(Sha256::digest(refresh_token.as_bytes()));
    assert_eq!(sessions[0].refresh_token_hash, expected_hash);
    assert_eq!(sessions[0].user_id, users[0].id);
    assert_eq!(sessions[0].provider, "mock");

    // The token's `sid` must name exactly the stored session's family: the
    // stable identity rotation never moves, so a presented access token
    // revokes precisely this credential chain and nothing else.
    assert!(
        is_valid_family_id(&claims.sid),
        "sid must be a well-formed family id, got {:?}",
        claims.sid
    );
    assert_eq!(
        claims.sid, sessions[0].family_id,
        "sid must be the stored session's family id"
    );

    // Exchange issues the family: generation 0, no rotation yet, and the
    // access token's `sid` names exactly that family.
    assert!(is_valid_family_id(&sessions[0].family_id));
    assert_eq!(sessions[0].generation, 0);
    assert_eq!(sessions[0].rotated_at, None);
    assert_eq!(
        claims.sid, sessions[0].family_id,
        "the sid claim must carry the session's stable family identifier"
    );
}

#[tokio::test]
async fn exchange_existing_user_does_not_create_new() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let svc = make_service(repo.clone(), provider);

    // First exchange: creates user
    let request1 = ExchangeRequest {
        provider_access_token: None,
        credential: ExchangeCredential::AuthorizationCode {
            code: "code-1".to_string(),
            redirect_uri: "https://app.test.com/callback".to_string(),
        },
        provider: "mock".to_string(),
        ip_address: None,
        user_agent: None,
        device_id: None,
    };
    let resp1 = svc
        .exchange(request1)
        .await
        .expect("first exchange should succeed");

    // Second exchange: same external_id, should reuse user
    let request2 = ExchangeRequest {
        provider_access_token: None,
        credential: ExchangeCredential::AuthorizationCode {
            code: "code-2".to_string(),
            redirect_uri: "https://app.test.com/callback".to_string(),
        },
        provider: "mock".to_string(),
        ip_address: None,
        user_agent: None,
        device_id: None,
    };
    let resp2 = svc
        .exchange(request2)
        .await
        .expect("second exchange should succeed");

    // Still only one user
    let users = repo.get_all_users().await;
    assert_eq!(users.len(), 1);

    // But two sessions
    let sessions = repo.get_all_sessions().await;
    assert_eq!(sessions.len(), 2);

    // Both tokens should reference the same user
    let payload1 = URL_SAFE_NO_PAD
        .decode(resp1.access_token.split('.').nth(1).unwrap())
        .unwrap();
    let claims1: AccessTokenClaims = serde_json::from_slice(&payload1).unwrap();

    let payload2 = URL_SAFE_NO_PAD
        .decode(resp2.access_token.split('.').nth(1).unwrap())
        .unwrap();
    let claims2: AccessTokenClaims = serde_json::from_slice(&payload2).unwrap();

    assert_eq!(claims1.sub, claims2.sub);

    // Each token binds to its own session family: the two sign-ins create
    // distinct families, so the sids must differ even though the subject
    // matches.
    let hash1 = hex::encode(Sha256::digest(
        resp1
            .refresh_token
            .expect("first exchange should return a refresh token")
            .as_bytes(),
    ));
    let hash2 = hex::encode(Sha256::digest(
        resp2
            .refresh_token
            .expect("second exchange should return a refresh token")
            .as_bytes(),
    ));
    let sessions = repo.get_all_sessions().await;
    let family_for = |hash: &str| {
        sessions
            .iter()
            .find(|s| s.refresh_token_hash == hash)
            .expect("session stored for hash")
            .family_id
            .clone()
    };
    assert_eq!(
        claims1.sid,
        family_for(&hash1),
        "first token's sid must be its own session's family"
    );
    assert_eq!(
        claims2.sid,
        family_for(&hash2),
        "second token's sid must be its own session's family"
    );
    assert_ne!(
        claims1.sid, claims2.sid,
        "separate exchanges must mint tokens for separate families"
    );
}

#[tokio::test]
async fn exchange_suspended_user_is_rejected() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let svc = make_service(repo.clone(), provider);

    // First exchange creates the user
    let request = ExchangeRequest {
        provider_access_token: None,
        credential: ExchangeCredential::AuthorizationCode {
            code: "code".to_string(),
            redirect_uri: "https://app.test.com/callback".to_string(),
        },
        provider: "mock".to_string(),
        ip_address: None,
        user_agent: None,
        device_id: None,
    };
    svc.exchange(request)
        .await
        .expect("first exchange should succeed");

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

    // Second exchange should fail
    let request2 = ExchangeRequest {
        provider_access_token: None,
        credential: ExchangeCredential::AuthorizationCode {
            code: "code-2".to_string(),
            redirect_uri: "https://app.test.com/callback".to_string(),
        },
        provider: "mock".to_string(),
        ip_address: None,
        user_agent: None,
        device_id: None,
    };
    let err = svc
        .exchange(request2)
        .await
        .expect_err("exchange should fail for suspended user");

    match err {
        Error::UserSuspended { user_id: id } => {
            assert_eq!(id, user_id);
        }
        other => panic!("expected UserSuspended, got: {:?}", other),
    }
}

#[tokio::test]
async fn exchange_unknown_provider_is_rejected() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let svc = make_service(repo, provider);

    let request = ExchangeRequest {
        provider_access_token: None,
        credential: ExchangeCredential::AuthorizationCode {
            code: "code".to_string(),
            redirect_uri: "https://app.test.com/callback".to_string(),
        },
        provider: "nonexistent".to_string(),
        ip_address: None,
        user_agent: None,
        device_id: None,
    };
    let err = svc
        .exchange(request)
        .await
        .expect_err("exchange should fail for unknown provider");

    match err {
        Error::UnknownProvider { provider } => {
            assert_eq!(provider, "nonexistent");
        }
        other => panic!("expected UnknownProvider, got: {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Registration policy tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn exchange_domain_allowlist_rejects_non_matching_domain() {
    let config = Config::resolve(RawConfig {
        registration: RawRegistrationConfig {
            mode: "open".to_string(),
            domain_allowlist: Some(vec!["example.com".to_string()]),
        },
        ..base_raw_config()
    })
    .expect("test config should resolve");

    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    // Default claims have email = "test@example.com", change to a non-matching domain
    provider
        .set_claims(IdentityClaims {
            subject: "test-subject".to_string(),
            email: Some("user@other.com".to_string()),
            email_verified: Some(true),
            name: Some("Test User".to_string()),
            is_private_email: None,
            signing_alg: "RS256".to_string(),
            raw_claims: verified_raw_claims(),
        })
        .await;

    let svc = make_service_with_config(repo, provider, config);

    let request = ExchangeRequest {
        provider_access_token: None,
        credential: ExchangeCredential::AuthorizationCode {
            code: "code".to_string(),
            redirect_uri: "https://app.test.com/callback".to_string(),
        },
        provider: "mock".to_string(),
        ip_address: None,
        user_agent: None,
        device_id: None,
    };

    let err = svc
        .exchange(request)
        .await
        .expect_err("should reject non-matching domain");

    match err {
        Error::AccessDenied { .. } => {} // expected
        other => panic!("expected AccessDenied, got: {:?}", other),
    }
}

#[tokio::test]
async fn exchange_wildcard_subdomain_matching() {
    let base_config = || {
        Config::resolve(RawConfig {
            registration: RawRegistrationConfig {
                mode: "open".to_string(),
                domain_allowlist: Some(vec!["*.example.com".to_string()]),
            },
            ..base_raw_config()
        })
        .expect("test config should resolve")
    };

    // sub.example.com should be allowed
    {
        let repo = MockRepository::new();
        let provider = MockIdentityProvider::new("mock");
        provider
            .set_claims(IdentityClaims {
                subject: "subject-1".to_string(),
                email: Some("user@sub.example.com".to_string()),
                email_verified: Some(true),
                name: None,
                is_private_email: None,
                signing_alg: "RS256".to_string(),
                raw_claims: verified_raw_claims(),
            })
            .await;
        let svc = make_service_with_config(repo, provider, base_config());
        let request = ExchangeRequest {
            provider_access_token: None,
            credential: ExchangeCredential::AuthorizationCode {
                code: "code".to_string(),
                redirect_uri: "https://app.test.com/callback".to_string(),
            },
            provider: "mock".to_string(),
            ip_address: None,
            user_agent: None,
            device_id: None,
        };
        svc.exchange(request)
            .await
            .expect("sub.example.com should be allowed");
    }

    // a.b.example.com should be allowed
    {
        let repo = MockRepository::new();
        let provider = MockIdentityProvider::new("mock");
        provider
            .set_claims(IdentityClaims {
                subject: "subject-2".to_string(),
                email: Some("user@a.b.example.com".to_string()),
                email_verified: Some(true),
                name: None,
                is_private_email: None,
                signing_alg: "RS256".to_string(),
                raw_claims: verified_raw_claims(),
            })
            .await;
        let svc = make_service_with_config(repo, provider, base_config());
        let request = ExchangeRequest {
            provider_access_token: None,
            credential: ExchangeCredential::AuthorizationCode {
                code: "code".to_string(),
                redirect_uri: "https://app.test.com/callback".to_string(),
            },
            provider: "mock".to_string(),
            ip_address: None,
            user_agent: None,
            device_id: None,
        };
        svc.exchange(request)
            .await
            .expect("a.b.example.com should be allowed");
    }

    // example.com itself should be rejected (wildcard requires subdomain)
    {
        let repo = MockRepository::new();
        let provider = MockIdentityProvider::new("mock");
        provider
            .set_claims(IdentityClaims {
                subject: "subject-3".to_string(),
                email: Some("user@example.com".to_string()),
                email_verified: Some(true),
                name: None,
                is_private_email: None,
                signing_alg: "RS256".to_string(),
                raw_claims: verified_raw_claims(),
            })
            .await;
        let svc = make_service_with_config(repo, provider, base_config());
        let request = ExchangeRequest {
            provider_access_token: None,
            credential: ExchangeCredential::AuthorizationCode {
                code: "code".to_string(),
                redirect_uri: "https://app.test.com/callback".to_string(),
            },
            provider: "mock".to_string(),
            ip_address: None,
            user_agent: None,
            device_id: None,
        };
        let err = svc
            .exchange(request)
            .await
            .expect_err("example.com should be rejected by wildcard");
        match err {
            Error::AccessDenied { .. } => {}
            other => panic!("expected AccessDenied, got: {:?}", other),
        }
    }

    // notexample.com should be rejected
    {
        let repo = MockRepository::new();
        let provider = MockIdentityProvider::new("mock");
        provider
            .set_claims(IdentityClaims {
                subject: "subject-4".to_string(),
                email: Some("user@notexample.com".to_string()),
                email_verified: Some(true),
                name: None,
                is_private_email: None,
                signing_alg: "RS256".to_string(),
                raw_claims: verified_raw_claims(),
            })
            .await;
        let svc = make_service_with_config(repo, provider, base_config());
        let request = ExchangeRequest {
            provider_access_token: None,
            credential: ExchangeCredential::AuthorizationCode {
                code: "code".to_string(),
                redirect_uri: "https://app.test.com/callback".to_string(),
            },
            provider: "mock".to_string(),
            ip_address: None,
            user_agent: None,
            device_id: None,
        };
        let err = svc
            .exchange(request)
            .await
            .expect_err("notexample.com should be rejected");
        match err {
            Error::AccessDenied { .. } => {}
            other => panic!("expected AccessDenied, got: {:?}", other),
        }
    }
}

#[tokio::test]
async fn exchange_existing_users_only_rejects_new_user() {
    let config = Config::resolve(RawConfig {
        registration: RawRegistrationConfig {
            mode: "existing_users_only".to_string(),
            domain_allowlist: None,
        },
        ..base_raw_config()
    })
    .expect("test config should resolve");

    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let svc = make_service_with_config(repo, provider, config);

    let request = ExchangeRequest {
        provider_access_token: None,
        credential: ExchangeCredential::AuthorizationCode {
            code: "code".to_string(),
            redirect_uri: "https://app.test.com/callback".to_string(),
        },
        provider: "mock".to_string(),
        ip_address: None,
        user_agent: None,
        device_id: None,
    };

    let err = svc
        .exchange(request)
        .await
        .expect_err("should reject new user in existing_users_only mode");

    match err {
        Error::AccessDenied { .. } => {} // expected
        other => panic!("expected AccessDenied, got: {:?}", other),
    }
}

#[tokio::test]
async fn exchange_existing_user_is_denied_after_allowlist_tightening() {
    // Configure an allowlist that does NOT include the current assertion's domain.
    let config = Config::resolve(RawConfig {
        registration: RawRegistrationConfig {
            mode: "open".to_string(),
            domain_allowlist: Some(vec!["allowed-only.com".to_string()]),
        },
        ..base_raw_config()
    })
    .expect("test config should resolve");

    let repo = MockRepository::new();

    // Pre-create the user in the repository (simulating an existing user)
    // The mock provider will return claims with subject "test-subject" and
    // email "test@example.com" — a domain NOT in the allowlist.
    repo.create_user(&NewUser {
        external_id: "test-subject".to_string(),
        provider: "mock".to_string(),
        email: Some("test@example.com".to_string()),
        display_name: Some("Test User".to_string()),
    })
    .await
    .expect("pre-create user should succeed");

    let provider = MockIdentityProvider::new("mock");
    let svc = make_service_with_config(repo, provider, config);

    let request = ExchangeRequest {
        provider_access_token: None,
        credential: ExchangeCredential::AuthorizationCode {
            code: "code".to_string(),
            redirect_uri: "https://app.test.com/callback".to_string(),
        },
        provider: "mock".to_string(),
        ip_address: None,
        user_agent: None,
        device_id: None,
    };

    let err = svc
        .exchange(request)
        .await
        .expect_err("existing user outside a tightened allowlist must be denied");
    assert!(matches!(err, Error::AccessDenied { .. }));
}

#[tokio::test]
async fn exchange_open_registration_requires_verified_email_without_allowlist() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    provider
        .set_claims(IdentityClaims {
            subject: "unverified-subject".to_string(),
            email: Some("user@example.com".to_string()),
            email_verified: Some(false),
            name: None,
            is_private_email: None,
            signing_alg: "RS256".to_string(),
            raw_claims: verified_raw_claims(),
        })
        .await;
    let svc = make_service(repo.clone(), provider);

    let err = svc
        .exchange(ExchangeRequest {
            credential: ExchangeCredential::AuthorizationCode {
                code: "code".to_string(),
                redirect_uri: "https://app.test.com/callback".to_string(),
            },
            provider: "mock".to_string(),
            provider_access_token: None,
            ip_address: None,
            user_agent: None,
            device_id: None,
        })
        .await
        .expect_err("open registration must reject an unverified email");

    assert!(matches!(err, Error::AccessDenied { .. }));
    assert!(repo.get_all_users().await.is_empty());
}

#[tokio::test]
async fn exchange_no_email_rejected_when_allowlist_configured() {
    let config = Config::resolve(RawConfig {
        registration: RawRegistrationConfig {
            mode: "open".to_string(),
            domain_allowlist: Some(vec!["example.com".to_string()]),
        },
        ..base_raw_config()
    })
    .expect("test config should resolve");

    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    // Set claims with no email
    provider
        .set_claims(IdentityClaims {
            subject: "test-subject-no-email".to_string(),
            email: None,
            email_verified: None,
            name: Some("No Email User".to_string()),
            is_private_email: None,
            signing_alg: "RS256".to_string(),
            raw_claims: verified_raw_claims(),
        })
        .await;

    let svc = make_service_with_config(repo, provider, config);

    let request = ExchangeRequest {
        provider_access_token: None,
        credential: ExchangeCredential::AuthorizationCode {
            code: "code".to_string(),
            redirect_uri: "https://app.test.com/callback".to_string(),
        },
        provider: "mock".to_string(),
        ip_address: None,
        user_agent: None,
        device_id: None,
    };

    let err = svc
        .exchange(request)
        .await
        .expect_err("should reject when no email and allowlist is configured");

    match err {
        Error::AccessDenied { .. } => {} // expected
        other => panic!("expected AccessDenied, got: {:?}", other),
    }
}

#[tokio::test]
async fn exchange_with_direct_id_token_skips_code_exchange() {
    let repo = MockRepository::new();

    // The direct grant requires a server-minted nonce echoed back inside the
    // assertion. Mint one through a throwaway service over the same repo
    // (minting touches only the single-use store), then pin claims that carry
    // it before building the service under test.
    let minter = make_service(repo.clone(), MockIdentityProvider::new("mock"));
    let minted = minter
        .mint_nonce()
        .await
        .expect("mint nonce should succeed");
    assert_eq!(minted.nonce.len(), 43, "nonce is 32 bytes base64url-no-pad");
    assert!(minted.expires_in > 0);

    let provider = MockIdentityProvider::new("mock");
    let mut raw = verified_raw_claims();
    raw.insert("nonce".to_string(), serde_json::json!(minted.nonce));
    provider
        .set_claims(IdentityClaims {
            subject: "test-subject".to_string(),
            email: Some("test@example.com".to_string()),
            email_verified: Some(true),
            name: Some("Test User".to_string()),
            is_private_email: None,
            signing_alg: "RS256".to_string(),
            raw_claims: raw,
        })
        .await;

    let svc = make_service(repo.clone(), provider);

    // Use id_token grant — no code or redirect_uri needed
    let request = ExchangeRequest {
        provider_access_token: None,
        credential: ExchangeCredential::IdTokenAssertion {
            id_token: "fake.id.token".to_string(),
        },
        provider: "mock".to_string(),
        ip_address: None,
        user_agent: None,
        device_id: None,
    };

    let result = svc
        .exchange(request)
        .await
        .expect("id_token exchange should succeed");

    assert!(!result.access_token.is_empty());
    assert!(result.refresh_token.is_some());
    assert_eq!(result.token_type, "Bearer");

    // Verify user was created
    let users = repo.get_all_users().await;
    assert_eq!(users.len(), 1);
    assert_eq!(users[0].external_id, "test-subject");
}

/// A missing credential is not testable here anymore: `ExchangeRequest`
/// carries an `ExchangeCredential` with no default, so "neither code nor
/// id_token" does not compile — the negative space moved from a runtime
/// branch to the type system (see task 01 of the grant-binding plan). The
/// HTTP-boundary equivalent lives in `crates/server/tests/routes.rs`.

#[tokio::test]
async fn exchange_conflict_on_create_re_lookups_and_returns_token() {
    let repo = MockRepository::new();

    // Racer A: a normal first login that wins the race and creates the user.
    let provider_a = MockIdentityProvider::new("mock");
    let svc_a = make_service(repo.clone(), provider_a);
    let request_a = ExchangeRequest {
        provider_access_token: None,
        credential: ExchangeCredential::AuthorizationCode {
            code: "code-a".to_string(),
            redirect_uri: "https://app.test.com/callback".to_string(),
        },
        provider: "mock".to_string(),
        ip_address: None,
        user_agent: None,
        device_id: None,
    };
    let resp_a = svc_a
        .exchange(request_a)
        .await
        .expect("winning racer should succeed");

    // Racer B: its first `get_user_by_external_id` reports "not found" (it
    // read before A's write committed), so it proceeds to `create_user`,
    // which conflicts against the real shared repository — exercising the
    // re-lookup path.
    let provider_b = MockIdentityProvider::new("mock");
    let stale_repo = StaleReadUserRepository::new(repo.clone(), 1);
    let audit_b = MockAuditLog::new();
    let audit_b_clone = audit_b.clone();
    let provider_id = provider_b.provider_id().to_string();
    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    providers.insert(provider_id, Box::new(provider_b));
    let svc_b = AppService::new(
        Box::new(stale_repo),
        Box::new(repo.clone()),
        Box::new(MockKeyManager::new()),
        Box::new(audit_b),
        Box::new(MockUserSync::new()),
        Box::new(oidc_exchange_test_utils::MockRateLimiter::new()),
        providers,
        make_config(),
    );
    let request_b = ExchangeRequest {
        provider_access_token: None,
        credential: ExchangeCredential::AuthorizationCode {
            code: "code-b".to_string(),
            redirect_uri: "https://app.test.com/callback".to_string(),
        },
        provider: "mock".to_string(),
        ip_address: None,
        user_agent: None,
        device_id: None,
    };
    let resp_b = svc_b
        .exchange(request_b)
        .await
        .expect("losing racer should still return a token via re-lookup, not a 500");

    // Both racers got a usable token, and exactly one user exists.
    assert!(!resp_a.access_token.is_empty());
    assert!(!resp_b.access_token.is_empty());
    let users = repo.get_all_users().await;
    assert_eq!(
        users.len(),
        1,
        "exactly one user should be created despite two racing exchanges"
    );

    // The losing racer must not emit a duplicate UserCreated audit event.
    // (The winner uses a separate audit log, so this captures only racer B.)
    // Both tokens reference the same, single user.
    let sub_a = decode_sub(&resp_a.access_token);
    let sub_b = decode_sub(&resp_b.access_token);
    assert_eq!(sub_a, sub_b);
    assert_eq!(sub_a, users[0].id);
    let events_b = audit_b_clone.events().await;
    assert_eq!(events_b.len(), 1);
    assert_eq!(events_b[0].event_type, AuditEventType::TokenExchange);
    assert_eq!(events_b[0].severity, AuditSeverity::Info);
}

#[tokio::test]
async fn exchange_conflict_re_lookup_reapplies_suspended_check() {
    let repo = MockRepository::new();

    // Racer A wins and creates the user; it is then suspended before racer
    // B's re-lookup observes it.
    let provider_a = MockIdentityProvider::new("mock");
    let svc_a = make_service(repo.clone(), provider_a);
    let request_a = ExchangeRequest {
        provider_access_token: None,
        credential: ExchangeCredential::AuthorizationCode {
            code: "code-a".to_string(),
            redirect_uri: "https://app.test.com/callback".to_string(),
        },
        provider: "mock".to_string(),
        ip_address: None,
        user_agent: None,
        device_id: None,
    };
    svc_a
        .exchange(request_a)
        .await
        .expect("winning racer should succeed");

    let winner_id = repo.get_all_users().await[0].id.clone();
    repo.update_user(
        &winner_id,
        &UserPatch {
            status: Some(UserStatus::Suspended),
            email: None,
            display_name: None,
            metadata: None,
            claims: None,
        },
    )
    .await
    .expect("suspend should succeed");

    // Racer B races in, conflicts, re-looks-up, and must re-apply the
    // suspended check against the winner it just found.
    let provider_b = MockIdentityProvider::new("mock");
    let stale_repo = StaleReadUserRepository::new(repo.clone(), 1);
    let svc_b = make_service_with_user_repo(
        Box::new(stale_repo),
        repo.clone(),
        provider_b,
        make_config(),
    );
    let request_b = ExchangeRequest {
        provider_access_token: None,
        credential: ExchangeCredential::AuthorizationCode {
            code: "code-b".to_string(),
            redirect_uri: "https://app.test.com/callback".to_string(),
        },
        provider: "mock".to_string(),
        ip_address: None,
        user_agent: None,
        device_id: None,
    };
    let err = svc_b
        .exchange(request_b)
        .await
        .expect_err("re-lookup of a suspended winner must reject, not issue a token");

    match err {
        Error::UserSuspended { user_id } => assert_eq!(user_id, winner_id),
        other => panic!("expected UserSuspended, got: {:?}", other),
    }

    // No second user was created, and the rejected racer did not mint a
    // session on top of racer A's single one.
    assert_eq!(repo.get_all_users().await.len(), 1);
    assert_eq!(repo.get_all_sessions().await.len(), 1);
}

#[tokio::test]
async fn exchange_non_conflict_create_error_propagates_without_relookup() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");

    // `create_user` always fails with a non-`Conflict` error (simulating a
    // real infrastructure failure); the exchange must propagate it directly
    // rather than treating it as a race and silently re-looking up.
    let (failing_repo, lookup_calls) = FailingCreateUserRepository::new(repo.clone());
    let audit = MockAuditLog::new();
    let audit_clone = audit.clone();
    let svc = make_service_with_user_repo_and_audit(
        Box::new(failing_repo),
        repo.clone(),
        provider,
        make_config(),
        audit,
    );

    let request = ExchangeRequest {
        provider_access_token: None,
        credential: ExchangeCredential::AuthorizationCode {
            code: "code".to_string(),
            redirect_uri: "https://app.test.com/callback".to_string(),
        },
        provider: "mock".to_string(),
        ip_address: None,
        user_agent: None,
        device_id: None,
    };
    let err = svc
        .exchange(request)
        .await
        .expect_err("a non-Conflict create_user error must propagate");

    match err {
        Error::StoreError { .. } => {}
        other => panic!("expected StoreError to propagate, got: {:?}", other),
    }

    // Infrastructure failures are not client-attributable outcomes: they are
    // not recorded as authentication failures (the failing store may be the
    // audit dependency itself), so no terminal event is emitted at all.
    let events = audit_clone.events().await;
    assert!(
        events.is_empty(),
        "an infrastructure failure must not be recorded as an authentication outcome: {:?}",
        events.iter().map(|e| e.event_type.clone()).collect::<Vec<_>>()
    );

    // No user or session was created, and the flow did not swallow the
    // infra error to attempt a silent re-lookup.
    assert_eq!(repo.get_all_users().await.len(), 0);
    assert_eq!(repo.get_all_sessions().await.len(), 0);

    // Exactly one lookup happened (the initial miss); a buggy implementation
    // that re-looks-up on *any* create_user error would call this twice.
    assert_eq!(
        lookup_calls.load(Ordering::SeqCst),
        1,
        "a non-Conflict create_user error must not trigger a re-lookup"
    );
}

#[tokio::test]
async fn exchange_provider_timeout_emits_exactly_one_typed_terminal_event() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    provider.set_exchange_timeout(true).await;
    let audit = MockAuditLog::new();
    let audit_clone = audit.clone();
    let svc = make_service_with_audit(repo, provider, make_config(), audit);

    let request = ExchangeRequest {
        credential: ExchangeCredential::AuthorizationCode {
            code: "code".to_string(),
            redirect_uri: "https://app.test.com/callback".to_string(),
        },
        provider: "mock".to_string(),
        provider_access_token: None,
        ip_address: None,
        user_agent: None,
        device_id: None,
    };
    let err = svc
        .exchange(request)
        .await
        .expect_err("provider must reject");
    assert!(matches!(err, Error::ProviderTimeout { .. }));

    let events = audit_clone.events().await;
    assert_eq!(events.len(), 1, "must emit exactly one terminal event");
    assert_eq!(events[0].event_type, AuditEventType::ProviderError);
    assert_eq!(
        events[0].outcome,
        AuditOutcome::Failure(AuditFailure::ProviderRejected)
    );
}

/// An `ExchangeRequest` carrying client-context values (`ip_address`,
/// `user_agent`, `device_id`) stores those exact values on the resulting
/// session — no truncation, reordering, or substitution.
#[tokio::test]
async fn exchange_with_client_context_stores_exact_session_values() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let svc = make_service(repo.clone(), provider);

    let request = ExchangeRequest {
        credential: ExchangeCredential::AuthorizationCode {
            code: "auth-code-123".to_string(),
            redirect_uri: "https://app.test.com/callback".to_string(),
        },
        provider: "mock".to_string(),
        provider_access_token: None,
        ip_address: Some("203.0.113.7".to_string()),
        user_agent: Some("integration-test-agent/1.0".to_string()),
        device_id: Some("device-abc-123".to_string()),
    };

    svc.exchange(request)
        .await
        .expect("exchange should succeed");

    let sessions = repo.get_all_sessions().await;
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].ip_address.as_deref(), Some("203.0.113.7"));
    assert_eq!(
        sessions[0].user_agent.as_deref(),
        Some("integration-test-agent/1.0")
    );
    assert_eq!(sessions[0].device_id.as_deref(), Some("device-abc-123"));
}

/// Negative-space: an `ExchangeRequest` with all three client-context fields
/// `None` stores a session with `None` for each — no accidental default
/// substitution (e.g. falling back to an empty string instead of `None`).
#[tokio::test]
async fn exchange_without_client_context_stores_none_session_values() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let svc = make_service(repo.clone(), provider);

    let request = ExchangeRequest {
        credential: ExchangeCredential::AuthorizationCode {
            code: "auth-code-123".to_string(),
            redirect_uri: "https://app.test.com/callback".to_string(),
        },
        provider: "mock".to_string(),
        provider_access_token: None,
        ip_address: None,
        user_agent: None,
        device_id: None,
    };

    svc.exchange(request)
        .await
        .expect("exchange should succeed");

    let sessions = repo.get_all_sessions().await;
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].ip_address, None);
    assert_eq!(sessions[0].user_agent, None);
    assert_eq!(sessions[0].device_id, None);
}

// ---------------------------------------------------------------------------
// Audit emission tests
// ---------------------------------------------------------------------------

/// A brand-new user's exchange emits exactly `UserCreated` (notice, success)
/// followed by `TokenExchange` (info, success), both carrying the request's
/// ip/ua and no other event types.
#[tokio::test]
async fn exchange_new_user_emits_user_created_then_token_exchange() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let audit = MockAuditLog::new();
    let audit_clone = audit.clone();
    let svc = make_service_with_audit(repo, provider, make_config(), audit);

    let request = ExchangeRequest {
        credential: ExchangeCredential::AuthorizationCode {
            code: "auth-code-123".to_string(),
            redirect_uri: "https://app.test.com/callback".to_string(),
        },
        provider: "mock".to_string(),
        provider_access_token: None,
        ip_address: Some("203.0.113.9".to_string()),
        user_agent: Some("test-agent/2.0".to_string()),
        device_id: None,
    };

    svc.exchange(request)
        .await
        .expect("exchange should succeed");

    let events = audit_clone.events().await;
    assert_eq!(
        events.len(),
        2,
        "expected exactly UserCreated + TokenExchange, got: {:?}",
        events.iter().map(|e| &e.event_type).collect::<Vec<_>>()
    );

    assert_eq!(events[0].event_type, AuditEventType::UserCreated);
    assert_eq!(events[0].severity, AuditSeverity::Notice);
    assert_eq!(events[0].outcome, AuditOutcome::Success);
    assert_eq!(events[0].provider.as_deref(), Some("mock"));
    assert_eq!(events[0].ip_address.as_deref(), Some("203.0.113.9"));
    assert_eq!(events[0].user_agent.as_deref(), Some("test-agent/2.0"));
    assert!(
        events[0].actor.is_some(),
        "UserCreated should carry the new user's id as actor"
    );

    assert_eq!(events[1].event_type, AuditEventType::TokenExchange);
    assert_eq!(events[1].severity, AuditSeverity::Info);
    assert_eq!(events[1].outcome, AuditOutcome::Success);
    assert_eq!(events[1].provider.as_deref(), Some("mock"));
    assert_eq!(events[1].ip_address.as_deref(), Some("203.0.113.9"));
    assert_eq!(events[1].user_agent.as_deref(), Some("test-agent/2.0"));
    assert_eq!(
        events[1].actor, events[0].actor,
        "TokenExchange should carry the same actor as UserCreated"
    );
}

/// Negative space: a returning (already-existing) user's exchange emits only
/// `TokenExchange` — no `UserCreated` event for a user that already existed.
#[tokio::test]
async fn exchange_existing_user_emits_only_token_exchange() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let svc = make_service(repo.clone(), provider);

    // First exchange creates the user (audit log discarded here).
    let request1 = ExchangeRequest {
        provider_access_token: None,
        credential: ExchangeCredential::AuthorizationCode {
            code: "code-1".to_string(),
            redirect_uri: "https://app.test.com/callback".to_string(),
        },
        provider: "mock".to_string(),
        ip_address: None,
        user_agent: None,
        device_id: None,
    };
    svc.exchange(request1)
        .await
        .expect("first exchange should succeed");

    // Second exchange, now with an audit log we can inspect, should reuse
    // the existing user and emit only TokenExchange.
    let provider2 = MockIdentityProvider::new("mock");
    let audit = MockAuditLog::new();
    let audit_clone = audit.clone();
    let svc2 = make_service_with_audit(repo, provider2, make_config(), audit);

    let request2 = ExchangeRequest {
        credential: ExchangeCredential::AuthorizationCode {
            code: "code-2".to_string(),
            redirect_uri: "https://app.test.com/callback".to_string(),
        },
        provider: "mock".to_string(),
        provider_access_token: None,
        ip_address: Some("203.0.113.10".to_string()),
        user_agent: Some("test-agent/3.0".to_string()),
        device_id: None,
    };
    svc2.exchange(request2)
        .await
        .expect("second exchange should succeed");

    let events = audit_clone.events().await;
    assert_eq!(
        events.len(),
        1,
        "existing-user exchange must not emit UserCreated, got: {:?}",
        events.iter().map(|e| &e.event_type).collect::<Vec<_>>()
    );
    assert_eq!(events[0].event_type, AuditEventType::TokenExchange);
    assert_eq!(events[0].ip_address.as_deref(), Some("203.0.113.10"));
    assert_eq!(events[0].user_agent.as_deref(), Some("test-agent/3.0"));
}

/// A suspended user's exchange emits exactly one `UserSuspended` (warning,
/// failure) event carrying the request's ip/ua and the user's id as actor —
/// and nothing else (no `TokenExchange`).
#[tokio::test]
async fn exchange_suspended_user_emits_only_user_suspended_event() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let svc = make_service(repo.clone(), provider);

    // First exchange creates the user.
    let request = ExchangeRequest {
        provider_access_token: None,
        credential: ExchangeCredential::AuthorizationCode {
            code: "code".to_string(),
            redirect_uri: "https://app.test.com/callback".to_string(),
        },
        provider: "mock".to_string(),
        ip_address: None,
        user_agent: None,
        device_id: None,
    };
    svc.exchange(request)
        .await
        .expect("first exchange should succeed");

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
    .expect("suspend should succeed");

    let provider2 = MockIdentityProvider::new("mock");
    let audit = MockAuditLog::new();
    let audit_clone = audit.clone();
    let svc2 = make_service_with_audit(repo, provider2, make_config(), audit);

    let request2 = ExchangeRequest {
        credential: ExchangeCredential::AuthorizationCode {
            code: "code-2".to_string(),
            redirect_uri: "https://app.test.com/callback".to_string(),
        },
        provider: "mock".to_string(),
        provider_access_token: None,
        ip_address: Some("203.0.113.11".to_string()),
        user_agent: Some("test-agent/4.0".to_string()),
        device_id: None,
    };
    svc2.exchange(request2)
        .await
        .expect_err("exchange should fail for suspended user");

    let events = audit_clone.events().await;
    assert_eq!(
        events.len(),
        1,
        "suspended-user exchange must emit exactly one event, got: {:?}",
        events.iter().map(|e| &e.event_type).collect::<Vec<_>>()
    );
    assert_eq!(events[0].event_type, AuditEventType::UserSuspended);
    match &events[0].outcome {
        AuditOutcome::Failure(_) => {}
        other => panic!("expected Failure outcome, got: {:?}", other),
    }
    assert_eq!(events[0].actor.as_deref(), Some(user_id.as_str()));
    assert_eq!(events[0].ip_address.as_deref(), Some("203.0.113.11"));
    assert_eq!(events[0].user_agent.as_deref(), Some("test-agent/4.0"));
}

/// Negative space: a domain-allowlist rejection emits `RegistrationDenied`
/// (warning, failure) and does not proceed to emit `TokenExchange` — the
/// user is never created and no token is ever issued.
#[tokio::test]
async fn exchange_existing_user_allowlist_rejection_names_user_in_audit() {
    let config = Config::resolve(RawConfig {
        registration: RawRegistrationConfig {
            mode: "open".to_string(),
            domain_allowlist: Some(vec!["allowed.example".to_string()]),
        },
        ..base_raw_config()
    })
    .expect("test config should resolve");
    let repo = MockRepository::new();
    let user = repo
        .create_user(&NewUser {
            external_id: "test-subject".to_string(),
            provider: "mock".to_string(),
            email: Some("old@allowed.example".to_string()),
            display_name: None,
        })
        .await
        .expect("pre-create existing user");
    let provider = MockIdentityProvider::new("mock");
    provider
        .set_claims(IdentityClaims {
            subject: "test-subject".to_string(),
            email: Some("current@outside.example".to_string()),
            email_verified: Some(true),
            name: None,
            is_private_email: None,
            signing_alg: "RS256".to_string(),
            raw_claims: verified_raw_claims(),
        })
        .await;
    let audit = MockAuditLog::new();
    let audit_clone = audit.clone();
    let svc = make_service_with_audit(repo, provider, config, audit);

    let err = svc
        .exchange(ExchangeRequest {
            credential: ExchangeCredential::AuthorizationCode {
                code: "code".to_string(),
                redirect_uri: "https://app.test.com/callback".to_string(),
            },
            provider: "mock".to_string(),
            provider_access_token: None,
            ip_address: None,
            user_agent: None,
            device_id: None,
        })
        .await
        .expect_err("existing user outside current allowlist must be denied");

    assert!(matches!(err, Error::AccessDenied { .. }));
    let events = audit_clone.events().await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, AuditEventType::RegistrationDenied);
    assert_eq!(events[0].severity, AuditSeverity::Warning);
    assert!(matches!(events[0].outcome, AuditOutcome::Failure { .. }));
    assert_eq!(events[0].actor.as_deref(), Some(user.id.as_str()));
}

#[tokio::test]
async fn exchange_existing_users_only_rejection_emits_registration_denied() {
    let config = Config::resolve(RawConfig {
        registration: RawRegistrationConfig {
            mode: "existing_users_only".to_string(),
            domain_allowlist: None,
        },
        ..base_raw_config()
    })
    .expect("test config should resolve");
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let audit = MockAuditLog::new();
    let audit_clone = audit.clone();
    let svc = make_service_with_audit(repo.clone(), provider, config, audit);

    let err = svc
        .exchange(ExchangeRequest {
            credential: ExchangeCredential::AuthorizationCode {
                code: "code".to_string(),
                redirect_uri: "https://app.test.com/callback".to_string(),
            },
            provider: "mock".to_string(),
            provider_access_token: None,
            ip_address: None,
            user_agent: None,
            device_id: None,
        })
        .await
        .expect_err("new users must be denied in existing_users_only mode");

    assert!(matches!(err, Error::AccessDenied { .. }));
    assert!(repo.get_all_users().await.is_empty());
    let events = audit_clone.events().await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, AuditEventType::RegistrationDenied);
    assert_eq!(events[0].severity, AuditSeverity::Warning);
    assert!(matches!(events[0].outcome, AuditOutcome::Failure { .. }));
    assert_eq!(events[0].actor, None);
}

#[tokio::test]
async fn exchange_domain_allowlist_rejection_emits_registration_denied_and_no_token_exchange() {
    let config = Config::resolve(RawConfig {
        registration: RawRegistrationConfig {
            mode: "open".to_string(),
            domain_allowlist: Some(vec!["example.com".to_string()]),
        },
        ..base_raw_config()
    })
    .expect("test config should resolve");

    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    provider
        .set_claims(IdentityClaims {
            subject: "test-subject".to_string(),
            email: Some("user@other.com".to_string()),
            email_verified: Some(true),
            name: Some("Test User".to_string()),
            is_private_email: None,
            signing_alg: "RS256".to_string(),
            raw_claims: verified_raw_claims(),
        })
        .await;

    let audit = MockAuditLog::new();
    let audit_clone = audit.clone();
    let svc = make_service_with_audit(repo, provider, config, audit);

    let request = ExchangeRequest {
        credential: ExchangeCredential::AuthorizationCode {
            code: "code".to_string(),
            redirect_uri: "https://app.test.com/callback".to_string(),
        },
        provider: "mock".to_string(),
        provider_access_token: None,
        ip_address: Some("203.0.113.12".to_string()),
        user_agent: Some("test-agent/5.0".to_string()),
        device_id: None,
    };

    svc.exchange(request)
        .await
        .expect_err("should reject non-matching domain");

    let events = audit_clone.events().await;
    assert_eq!(
        events.len(),
        1,
        "allowlist rejection must emit exactly RegistrationDenied, got: {:?}",
        events.iter().map(|e| &e.event_type).collect::<Vec<_>>()
    );
    assert_eq!(events[0].event_type, AuditEventType::RegistrationDenied);
    assert_eq!(events[0].severity, AuditSeverity::Warning);
    match &events[0].outcome {
        AuditOutcome::Failure(_) => {}
        other => panic!("expected Failure outcome, got: {:?}", other),
    }
    assert_eq!(events[0].ip_address.as_deref(), Some("203.0.113.12"));
    assert_eq!(events[0].user_agent.as_deref(), Some("test-agent/5.0"));
    assert!(
        !events
            .iter()
            .any(|e| e.event_type == AuditEventType::TokenExchange),
        "a rejected exchange must never emit TokenExchange"
    );
}

/// An enforcing terminal audit failure revokes the newly stored session before
/// propagating. The existing user isolates terminal success from `UserCreated`.
#[tokio::test]
async fn exchange_enforce_audit_failure_revokes_new_session() {
    let config = Config::resolve(RawConfig {
        audit: oidc_exchange_core::config::RawAuditConfig {
            adapter: "noop".to_string(),
            durability: "enforce".to_string(),
            blocking_threshold: "warning".to_string(),
            emit_threshold: "info".to_string(),
            sqs: None,
        },
        ..base_raw_config()
    })
    .expect("test config should resolve");

    let repo = MockRepository::new();

    // Pre-create the user directly (bypassing `exchange`, so no audit event
    // is emitted yet) so the upcoming exchange takes the existing-user path
    // and never emits `UserCreated`.
    repo.create_user(&NewUser {
        external_id: "test-subject".to_string(),
        provider: "mock".to_string(),
        email: Some("test@example.com".to_string()),
        display_name: Some("Test User".to_string()),
    })
    .await
    .expect("pre-create user should succeed");

    let provider = MockIdentityProvider::new("mock");
    let audit = MockAuditLog::new();
    audit.set_fail_mode(true).await;
    let svc = make_service_with_audit(repo.clone(), provider, config, audit);

    let request = ExchangeRequest {
        provider_access_token: None,
        credential: ExchangeCredential::AuthorizationCode {
            code: "code".to_string(),
            redirect_uri: "https://app.test.com/callback".to_string(),
        },
        provider: "mock".to_string(),
        ip_address: None,
        user_agent: None,
        device_id: None,
    };

    let err = svc
        .exchange(request)
        .await
        .expect_err("a blocking audit failure must propagate as Err");

    match err {
        Error::SecurityAuditDurability { .. } => {}
        other => panic!("expected SecurityAuditDurability, got: {:?}", other),
    }

    assert!(
        repo.get_all_sessions().await.is_empty(),
        "an enforcing terminal audit failure must revoke the newly stored session"
    );
}

// ---------------------------------------------------------------------------
// JIT user-sync notify tests
// ---------------------------------------------------------------------------

/// A JIT-registered (first-login) user fires exactly one
/// `notify_user_created` sync call carrying the newly created user; a
/// second exchange for the same identity (now an existing, active user)
/// fires no further sync calls at all.
#[tokio::test]
async fn exchange_jit_registration_fires_exactly_one_user_created_notify() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let user_sync = MockUserSync::new();
    let sync_clone = user_sync.clone();
    let svc = make_service_with_user_sync(repo.clone(), provider, make_config(), user_sync);

    let request = ExchangeRequest {
        provider_access_token: None,
        credential: ExchangeCredential::AuthorizationCode {
            code: "auth-code-123".to_string(),
            redirect_uri: "https://app.test.com/callback".to_string(),
        },
        provider: "mock".to_string(),
        ip_address: None,
        user_agent: None,
        device_id: None,
    };
    svc.exchange(request)
        .await
        .expect("JIT exchange should succeed");

    let users = repo.get_all_users().await;
    assert_eq!(users.len(), 1);
    let created_user_id = users[0].id.clone();

    let calls = sync_clone.calls().await;
    assert_eq!(
        calls.len(),
        1,
        "a JIT-registered user must fire exactly one sync call, got: {:?}",
        calls
    );
    match &calls[0] {
        UserSyncCall::Created(u) => assert_eq!(u.id, created_user_id),
        other => panic!("expected UserSyncCall::Created, got: {:?}", other),
    }

    // A second exchange for the same identity (the mock provider always
    // returns the same "test-subject" claim, so this hits the now-existing
    // active user) must not fire a second notify_user_created. Reuse the
    // SAME service/mock so the second exchange's sync calls are actually
    // observed by `sync_clone` rather than routed to a throwaway mock.
    let request2 = ExchangeRequest {
        provider_access_token: None,
        credential: ExchangeCredential::AuthorizationCode {
            code: "auth-code-456".to_string(),
            redirect_uri: "https://app.test.com/callback".to_string(),
        },
        provider: "mock".to_string(),
        ip_address: None,
        user_agent: None,
        device_id: None,
    };
    svc.exchange(request2)
        .await
        .expect("second exchange for an existing user should succeed");

    // The same sync mock saw no additional calls beyond the single
    // JIT-registration notify from the first exchange.
    let calls_after = sync_clone.calls().await;
    assert_eq!(
        calls_after.len(),
        1,
        "an existing-user exchange must not trigger any further sync calls"
    );
    assert_eq!(
        repo.get_all_users().await.len(),
        1,
        "still exactly one user"
    );
}

/// Negative space: when the user-sync backend fails every attempt, a JIT
/// exchange still returns a usable token — the best-effort notify's failure
/// is swallowed (log-and-continue) and never propagates as an `Err`, nor
/// does it prevent the session/user from being persisted.
#[tokio::test]
async fn exchange_jit_registration_still_returns_token_when_sync_fails_every_attempt() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let user_sync = MockUserSync::new();
    user_sync.set_fail_mode(true).await;
    let sync_clone = user_sync.clone();
    let svc = make_service_with_user_sync(repo.clone(), provider, make_config(), user_sync);

    let request = ExchangeRequest {
        provider_access_token: None,
        credential: ExchangeCredential::AuthorizationCode {
            code: "auth-code-789".to_string(),
            redirect_uri: "https://app.test.com/callback".to_string(),
        },
        provider: "mock".to_string(),
        ip_address: None,
        user_agent: None,
        device_id: None,
    };
    let response = svc
        .exchange(request)
        .await
        .expect("exchange must succeed even though user sync fails every attempt");

    assert_eq!(response.token_type, "Bearer");
    assert!(
        response.refresh_token.is_some(),
        "a token must still be issued despite the sync failure"
    );

    // The user and session were still persisted — the sync failure is
    // isolated from the rest of the flow.
    assert_eq!(repo.get_all_users().await.len(), 1);
    assert_eq!(repo.get_all_sessions().await.len(), 1);

    // The failed call was never recorded as a successful sync (fail_mode
    // returns Err before pushing onto the call log).
    assert!(
        sync_clone.calls().await.is_empty(),
        "a failed sync attempt must not be recorded as a successful call"
    );
}

/// Decode the `sub` claim out of a signed access token's payload segment.
fn decode_sub(access_token: &str) -> String {
    let payload_bytes = URL_SAFE_NO_PAD
        .decode(access_token.split('.').nth(1).unwrap())
        .expect("payload should be valid base64url");
    let claims: AccessTokenClaims =
        serde_json::from_slice(&payload_bytes).expect("payload should deserialize");
    claims.sub
}
