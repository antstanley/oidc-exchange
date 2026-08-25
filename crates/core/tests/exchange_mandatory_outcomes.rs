use std::collections::HashMap;

use oidc_exchange_core::config::{Config, RawConfig};
use oidc_exchange_core::domain::{
    AuditEventType, AuditFailure, AuditOutcome, NewUser, RateLimitDecision, RateLimitKey,
    UserPatch, UserStatus,
};
use oidc_exchange_core::error::Error;
use oidc_exchange_core::ports::{IdentityProvider, UserRepository};
use oidc_exchange_core::service::exchange::{ExchangeCredential, ExchangeRequest};
use oidc_exchange_core::service::AppService;
use oidc_exchange_test_utils::{
    MockAuditLog, MockIdentityProvider, MockKeyManager, MockRateLimiter, MockRepository,
    MockUserSync,
};

fn base_raw() -> RawConfig {
    let mut raw: RawConfig = toml::from_str(include_str!("../../../config/default.toml"))
        .expect("default config deserializes");
    raw.server.issuer = "https://auth.test".to_string();
    raw
}

fn config() -> Config {
    Config::resolve(base_raw()).expect("test config resolves")
}

fn service(
    repo: MockRepository,
    provider: MockIdentityProvider,
    audit: MockAuditLog,
    limiter: MockRateLimiter,
    config: Config,
) -> AppService {
    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    providers.insert(provider.provider_id().to_owned(), Box::new(provider));
    AppService::new(
        Box::new(repo.clone()),
        Box::new(repo),
        Box::new(MockKeyManager::new()),
        Box::new(audit),
        Box::new(MockUserSync::new()),
        Box::new(limiter),
        providers,
        config,
    )
}

fn request() -> ExchangeRequest {
    ExchangeRequest {
        credential: ExchangeCredential::AuthorizationCode {
            code: "code".into(),
            redirect_uri: "https://app.test/callback".into(),
        },
        provider: "mock".into(),
        provider_access_token: None,
        client_addr: oidc_exchange_core::domain::ClientAddr::Unknown,
        user_agent: None,
        device_id: None,
    }
}

#[tokio::test]
async fn terminal_outcome_space_emits_exactly_one_safe_event() {
    // An incoherent grant/field combination is unrepresentable in the typed
    // `ExchangeCredential`, so the second reachable AuthenticationFailed shape
    // here is a provider-rejected credential (`invalid_grant`).
    let cases = [
        (
            "unknown provider",
            "unknown",
            AuditEventType::ValidationFailed,
        ),
        (
            "rejected credential",
            "mock",
            AuditEventType::ValidationFailed,
        ),
    ];

    for (name, provider_name, expected) in cases {
        let repo = MockRepository::new();
        let provider = MockIdentityProvider::new("mock");
        if name == "rejected credential" {
            provider.set_exchange_error("invalid_grant").await;
        }
        let audit = MockAuditLog::new();
        let audit_view = audit.clone();
        let svc = service(repo, provider, audit, MockRateLimiter::new(), config());
        let mut exchange_request = request();
        exchange_request.provider = provider_name.into();
        svc.exchange(exchange_request).await.expect_err(name);
        let events = audit_view.events().await;
        assert_eq!(events.len(), 1, "{name} must have one terminal event");
        assert_eq!(events[0].event_type, expected);
        assert!(matches!(events[0].outcome, AuditOutcome::Failure(_)));
    }
}

#[tokio::test]
async fn emergency_threshold_cannot_suppress_terminal_security_event() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let audit = MockAuditLog::new();
    let audit_view = audit.clone();
    let config = {
        let mut raw = base_raw();
        raw.audit.emit_threshold = "emergency".to_string();
        Config::resolve(raw).expect("test config resolves")
    };
    let svc = service(repo, provider, audit, MockRateLimiter::new(), config);
    let mut exchange_request = request();
    exchange_request.provider = "unknown".into();
    svc.exchange(exchange_request)
        .await
        .expect_err("unknown provider");
    let events = audit_view.events().await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, AuditEventType::ValidationFailed);
}

#[tokio::test]
async fn provider_denial_prevents_outbound_provider_work() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let audit = MockAuditLog::new();
    let audit_view = audit.clone();
    let limiter = MockRateLimiter::new();
    limiter
        .set_decisions(vec![RateLimitDecision::Deny {
            retry_after_secs: 7,
        }])
        .await;
    let provider_view = provider.clone();
    let svc = service(repo, provider, audit, limiter, config());
    let err = svc
        .exchange(request())
        .await
        .expect_err("provider throttled");
    assert!(matches!(
        err,
        Error::TooManyRequests {
            retry_after_secs: 7
        }
    ));
    assert_eq!(provider_view.exchange_code_call_count().await, 0);
    assert_eq!(provider_view.validate_id_token_call_count().await, 0);
    let events = audit_view.events().await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, AuditEventType::ThrottleExceeded);
}

#[tokio::test]
async fn subject_denial_follows_validated_claims_and_prevents_session_write() {
    let repo = MockRepository::new();
    let provider = MockIdentityProvider::new("mock");
    let audit = MockAuditLog::new();
    let audit_view = audit.clone();
    let limiter = MockRateLimiter::new();
    limiter
        .set_decisions(vec![
            RateLimitDecision::Deny {
                retry_after_secs: 9,
            },
            RateLimitDecision::Allow,
        ])
        .await;
    let limiter_view = limiter.clone();
    let svc = service(repo.clone(), provider, audit, limiter, config());
    let err = svc
        .exchange(request())
        .await
        .expect_err("subject throttled");
    assert!(matches!(
        err,
        Error::TooManyRequests {
            retry_after_secs: 9
        }
    ));
    let keys = limiter_view.keys().await;
    assert!(matches!(
        keys.as_slice(),
        [RateLimitKey::Provider(_), RateLimitKey::Subject { .. }]
    ));
    assert!(repo.get_all_sessions().await.is_empty());
    let events = audit_view.events().await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, AuditEventType::ThrottleExceeded);
}

#[tokio::test]
async fn mandatory_failure_paths_emit_exactly_one_expected_event() {
    enum Case {
        ProviderRejection,
        RegistrationDenial,
        Suspension,
        ProviderLimiter,
        SubjectLimiter,
    }

    for case in [
        Case::ProviderRejection,
        Case::RegistrationDenial,
        Case::Suspension,
        Case::ProviderLimiter,
        Case::SubjectLimiter,
    ] {
        let repo = MockRepository::new();
        let provider = MockIdentityProvider::new("mock");
        let audit = MockAuditLog::new();
        let audit_view = audit.clone();
        let limiter = MockRateLimiter::new();
        let (expected, expected_failure, label) = match case {
            Case::ProviderRejection => {
                provider.set_exchange_error("rejected").await;
                (
                    AuditEventType::ProviderError,
                    AuditFailure::ProviderRejected,
                    "provider rejection",
                )
            }
            Case::RegistrationDenial => (
                AuditEventType::RegistrationDenied,
                AuditFailure::RegistrationDenied,
                "registration denial",
            ),
            Case::Suspension => {
                let user = repo
                    .create_user(&NewUser {
                        external_id: "test-subject".into(),
                        provider: "mock".into(),
                        email: Some("test@example.com".into()),
                        display_name: None,
                    })
                    .await
                    .unwrap();
                repo.update_user(
                    &user.id,
                    &UserPatch {
                        email: None,
                        display_name: None,
                        metadata: None,
                        claims: None,
                        status: Some(UserStatus::Suspended),
                    },
                )
                .await
                .unwrap();
                (
                    AuditEventType::UserSuspended,
                    AuditFailure::PrincipalSuspended,
                    "suspension",
                )
            }
            Case::ProviderLimiter => {
                limiter
                    .set_decisions(vec![RateLimitDecision::Deny {
                        retry_after_secs: 1,
                    }])
                    .await;
                (
                    AuditEventType::ThrottleExceeded,
                    AuditFailure::ThrottleExceeded,
                    "provider limiter",
                )
            }
            Case::SubjectLimiter => {
                limiter
                    .set_decisions(vec![
                        RateLimitDecision::Allow,
                        RateLimitDecision::Deny {
                            retry_after_secs: 1,
                        },
                    ])
                    .await;
                (
                    AuditEventType::ThrottleExceeded,
                    AuditFailure::ThrottleExceeded,
                    "subject limiter",
                )
            }
        };
        let cfg = if matches!(case, Case::RegistrationDenial) {
            let mut raw = base_raw();
            raw.registration.mode = "existing_users_only".to_string();
            Config::resolve(raw).expect("test config resolves")
        } else {
            config()
        };
        let svc = service(repo, provider, audit, limiter, cfg);
        svc.exchange(request()).await.expect_err(label);
        let events = audit_view.events().await;
        assert_eq!(events.len(), 1, "{label} must emit exactly one event");
        assert_eq!(events[0].event_type, expected, "{label}");
        assert_eq!(
            events[0].outcome,
            AuditOutcome::Failure(expected_failure),
            "{label}"
        );
    }
}

#[tokio::test]
async fn enforce_failure_removes_session_while_observe_failure_remains_visible() {
    for (durability, sessions) in [("enforce", 0), ("observe", 1)] {
        let repo = MockRepository::new();
        repo.create_user(&NewUser {
            external_id: "test-subject".into(),
            provider: "mock".into(),
            email: Some("test@example.com".into()),
            display_name: None,
        })
        .await
        .unwrap();
        let provider = MockIdentityProvider::new("mock");
        let audit = MockAuditLog::new();
        audit.set_fail_mode(true).await;
        let cfg = {
            let mut raw = base_raw();
            raw.audit.durability = durability.to_string();
            Config::resolve(raw).expect("test config resolves")
        };
        let svc = service(repo.clone(), provider, audit, MockRateLimiter::new(), cfg);
        let result = svc.exchange(request()).await;
        assert_eq!(result.is_err(), durability == "enforce");
        assert_eq!(repo.get_all_sessions().await.len(), sessions);
    }
}
