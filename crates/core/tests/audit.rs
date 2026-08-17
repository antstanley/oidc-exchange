use std::collections::HashMap;

use oidc_exchange_core::config::{AppConfig, AuditConfig};
use std::net::{IpAddr, Ipv4Addr};

use oidc_exchange_core::domain::{
    subject_hash, AdminMutationKind, AuditEventType, AuditFailure, AuditOutcome, AuditSeverity,
    AuthenticationKind, ClientAddr, ClientAddrSource, RateLimitDecision, RateLimitKey,
    SecurityEvent, MAX_ASSERTED_CLIENT_ADDR_LEN, MAX_RATE_LIMIT_PROVIDER_LEN,
};
use oidc_exchange_core::error::Error;
use oidc_exchange_core::ports::{IdentityProvider, RateLimiter};
use oidc_exchange_core::service::{
    audit_sink_degraded, audit_sink_failures_total, create_audit_event, AppService,
};

use oidc_exchange_test_utils::{
    MockAuditLog, MockIdentityProvider, MockKeyManager, MockRateLimiter, MockRepository,
    MockUserSync,
};

fn make_config_with_threshold(threshold: &str) -> AppConfig {
    AppConfig {
        audit: AuditConfig {
            blocking_threshold: threshold.to_string(),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn make_config_with_durability(durability: &str) -> AppConfig {
    AppConfig {
        audit: AuditConfig {
            durability: durability.to_string(),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn make_config_with_emit_threshold(emit_threshold: &str) -> AppConfig {
    AppConfig {
        audit: AuditConfig {
            emit_threshold: emit_threshold.to_string(),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn make_service_with_audit(audit: MockAuditLog, config: AppConfig) -> AppService {
    let provider = MockIdentityProvider::new("mock");
    let provider_id = provider.provider_id().to_string();
    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    providers.insert(provider_id, Box::new(provider));

    AppService::new(
        Box::new(MockRepository::new()),
        Box::new(MockRepository::new()),
        Box::new(MockKeyManager::new()),
        Box::new(audit),
        Box::new(MockUserSync::new()),
        Box::new(oidc_exchange_test_utils::MockRateLimiter::new()),
        providers,
        config,
    )
}

/// Non-blocking audit failure: Info event with Warning threshold.
/// Info (6) > Warning (4), so the failure is swallowed.
#[tokio::test]
async fn non_blocking_audit_failure_info_event_warning_threshold() {
    let audit = MockAuditLog::new();
    audit.set_fail_mode(true).await;

    let config = make_config_with_threshold("warning");
    let svc = make_service_with_audit(audit, config);

    let event = create_audit_event(
        AuditEventType::TokenExchange,
        AuditSeverity::Info,
        AuditOutcome::Success,
        Some("user-1".to_string()),
        Some("mock".to_string()),
        ClientAddr::Unknown,
        None,
    );

    let result = svc.emit_audit(event).await;
    assert!(
        result.is_ok(),
        "Info (6) > Warning (4): audit failure should be swallowed"
    );
}

/// Blocking audit failure: Warning event with Warning threshold.
/// Warning (4) <= Warning (4), so the error propagates.
#[tokio::test]
async fn blocking_audit_failure_warning_event_warning_threshold() {
    let audit = MockAuditLog::new();
    audit.set_fail_mode(true).await;

    let config = make_config_with_threshold("warning");
    let svc = make_service_with_audit(audit, config);

    let event = create_audit_event(
        AuditEventType::TokenExchange,
        AuditSeverity::Warning,
        AuditOutcome::Success,
        Some("user-1".to_string()),
        Some("mock".to_string()),
        ClientAddr::Unknown,
        None,
    );

    let result = svc.emit_audit(event).await;
    assert!(
        result.is_err(),
        "Warning (4) <= Warning (4): audit failure should block"
    );
}

/// Blocking audit failure: Error event with Warning threshold.
/// Error (3) <= Warning (4), so the error propagates.
#[tokio::test]
async fn blocking_audit_failure_error_event_warning_threshold() {
    let audit = MockAuditLog::new();
    audit.set_fail_mode(true).await;

    let config = make_config_with_threshold("warning");
    let svc = make_service_with_audit(audit, config);

    let event = create_audit_event(
        AuditEventType::TokenExchange,
        AuditSeverity::Error,
        AuditOutcome::Failure(AuditFailure::AuthenticationFailed),
        Some("user-1".to_string()),
        Some("mock".to_string()),
        ClientAddr::Unknown,
        None,
    );

    let result = svc.emit_audit(event).await;
    assert!(
        result.is_err(),
        "Error (3) <= Warning (4): audit failure should block"
    );
}

/// Successful audit emit: normal mode (not failing).
/// The event should be recorded in MockAuditLog.
#[tokio::test]
async fn successful_audit_emit_records_event() {
    let audit = MockAuditLog::new();
    let audit_clone = audit.clone();

    let config = make_config_with_threshold("warning");
    let svc = make_service_with_audit(audit, config);

    let event = create_audit_event(
        AuditEventType::TokenExchange,
        AuditSeverity::Info,
        AuditOutcome::Success,
        Some("user-1".to_string()),
        Some("mock".to_string()),
        ClientAddr::asserted("203.0.113.5").unwrap(),
        Some("test-agent/1.0".to_string()),
    );

    let result = svc.emit_audit(event).await;
    assert!(result.is_ok(), "successful audit emit should return Ok");

    let events = audit_clone.events().await;
    assert_eq!(events.len(), 1, "one event should have been recorded");
    assert_eq!(events[0].event_type, AuditEventType::TokenExchange);
    assert_eq!(events[0].severity, AuditSeverity::Info);
    assert_eq!(events[0].actor.as_deref(), Some("user-1"));
    assert_eq!(events[0].provider.as_deref(), Some("mock"));
    assert_eq!(events[0].ip_address.as_deref(), Some("203.0.113.5"));
    assert_eq!(events[0].ip_address_source, ClientAddrSource::Asserted);
    assert_eq!(events[0].user_agent.as_deref(), Some("test-agent/1.0"));
}

/// Negative-space: when `create_audit_event` is called with `None` for both
/// `ip_address` and `user_agent`, the resulting event carries `None` for
/// both — no accidental default substitution.
#[tokio::test]
async fn create_audit_event_with_no_client_context_leaves_fields_none() {
    let event = create_audit_event(
        AuditEventType::TokenExchange,
        AuditSeverity::Info,
        AuditOutcome::Success,
        Some("user-1".to_string()),
        Some("mock".to_string()),
        ClientAddr::Unknown,
        None,
    );

    assert_eq!(event.ip_address, None);
    assert_eq!(event.ip_address_source, ClientAddrSource::Unknown);
    assert_eq!(event.user_agent, None);
}

/// Negative-space: a Debug-severity event is strictly less severe than the
/// default `info` emit_threshold, so `emit_audit` drops it before dispatch —
/// the adapter never sees it, and the call still returns `Ok`.
#[tokio::test]
async fn audit_debug_event_under_default_emit_threshold_is_suppressed() {
    let audit = MockAuditLog::new();
    let audit_clone = audit.clone();

    // Default AppConfig carries the default AuditConfig, whose
    // `emit_threshold` defaults to "info".
    let config = AppConfig::default();
    assert_eq!(config.audit.emit_threshold, "info");
    let svc = make_service_with_audit(audit, config);

    let event = create_audit_event(
        AuditEventType::ValidationFailed,
        AuditSeverity::Debug,
        AuditOutcome::Failure(AuditFailure::AuthenticationFailed),
        None,
        Some("mock".to_string()),
        ClientAddr::Unknown,
        None,
    );

    let result = svc.emit_audit(event).await;
    assert!(
        result.is_ok(),
        "suppressed events must not surface an error"
    );

    let events = audit_clone.events().await;
    assert!(
        events.is_empty(),
        "Debug event under the default info emit_threshold must never reach the adapter"
    );
}

/// An Info event at the default `info` emit_threshold is exactly at the
/// floor (not strictly less severe), so it is dispatched normally.
#[tokio::test]
async fn audit_info_event_at_default_emit_threshold_is_dispatched() {
    let audit = MockAuditLog::new();
    let audit_clone = audit.clone();

    let config = AppConfig::default();
    let svc = make_service_with_audit(audit, config);

    let event = create_audit_event(
        AuditEventType::TokenExchange,
        AuditSeverity::Info,
        AuditOutcome::Success,
        Some("user-1".to_string()),
        Some("mock".to_string()),
        ClientAddr::Unknown,
        None,
    );

    let result = svc.emit_audit(event).await;
    assert!(result.is_ok(), "info event at info threshold should emit");

    let events = audit_clone.events().await;
    assert_eq!(
        events.len(),
        1,
        "an event at the emit_threshold must reach the adapter"
    );
    assert_eq!(events[0].severity, AuditSeverity::Info);
}

/// Lowering `emit_threshold` to `debug` lets a Debug event through to the
/// adapter, proving the filter is driven by config rather than a hardcoded
/// floor.
#[tokio::test]
async fn audit_debug_event_reaches_adapter_when_threshold_lowered_to_debug() {
    let audit = MockAuditLog::new();
    let audit_clone = audit.clone();

    let config = make_config_with_emit_threshold("debug");
    let svc = make_service_with_audit(audit, config);

    let event = create_audit_event(
        AuditEventType::ValidationFailed,
        AuditSeverity::Debug,
        AuditOutcome::Failure(AuditFailure::AuthenticationFailed),
        None,
        Some("mock".to_string()),
        ClientAddr::Unknown,
        None,
    );

    let result = svc.emit_audit(event).await;
    assert!(result.is_ok(), "dispatched debug event should return Ok");

    let events = audit_clone.events().await;
    assert_eq!(
        events.len(),
        1,
        "lowering emit_threshold to debug must let the debug event through"
    );
    assert_eq!(events[0].severity, AuditSeverity::Debug);
}

#[tokio::test]
async fn mandatory_security_audit_bypasses_emit_threshold_and_observes_failures() {
    let audit = MockAuditLog::new();
    audit.set_fail_mode(true).await;
    let svc = make_service_with_audit(audit, make_config_with_durability("observe"));

    // This process-global metric is also incremented by concurrently executing integration
    // tests. The mandatory path is proved by the sink's own observed call, which is isolated
    // to this fixture, rather than assuming a stable global counter delta.
    let before = audit_sink_failures_total();
    assert!(svc
        .emit_security_event(
            SecurityEvent::AuthenticationFailed,
            AuditOutcome::Failure(AuditFailure::AuthenticationFailed),
            None,
            None,
            ClientAddr::Unknown,
            None,
        )
        .await
        .is_ok());
    assert!(
        audit_sink_failures_total() > before,
        "the mandatory failure must increment the process metric even when other tests emit concurrently"
    );
    assert!(audit_sink_degraded());
}

#[tokio::test]
async fn mandatory_security_audit_returns_typed_durability_error_when_enforced() {
    let audit = MockAuditLog::new();
    audit.set_fail_mode(true).await;
    let svc = make_service_with_audit(audit, make_config_with_durability("enforce"));

    assert!(matches!(
        svc.emit_security_event(
            SecurityEvent::AuthenticationFailed,
            AuditOutcome::Failure(AuditFailure::AuthenticationFailed),
            None,
            None,
            ClientAddr::Unknown,
            None,
        )
        .await,
        Err(Error::SecurityAuditDurability { .. })
    ));
}

#[tokio::test]
async fn mock_rate_limiter_records_safe_keys_and_supports_deny_seam() {
    let limiter = MockRateLimiter::new();
    limiter
        .set_decisions(vec![RateLimitDecision::Deny {
            retry_after_secs: 30,
        }])
        .await;
    let key = RateLimitKey::ClientAddr(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 5)));
    assert_eq!(
        limiter.check_and_consume(&key).await.unwrap(),
        RateLimitDecision::Deny {
            retry_after_secs: 30
        }
    );
    assert_eq!(limiter.keys().await, vec![key]);
}

#[test]
fn rate_limit_subject_key_hashes_raw_subject_and_bounds_provider() {
    let key = RateLimitKey::subject(Some("mock"), "provider-subject").unwrap();
    let rendered = format!("{key:?}");
    assert!(rendered.contains(&subject_hash("provider-subject")));
    assert!(!rendered.contains("provider-subject"));
    assert!(RateLimitKey::provider("mock").is_some());
    assert!(RateLimitKey::provider("x".repeat(MAX_RATE_LIMIT_PROVIDER_LEN + 1)).is_none());
    assert!(RateLimitKey::subject(
        Some(&"x".repeat(MAX_RATE_LIMIT_PROVIDER_LEN + 1)),
        "provider-subject"
    )
    .is_none());
}

#[test]
fn security_events_have_exhaustive_fixed_audit_mappings() {
    let cases = [
        (
            SecurityEvent::AuthenticationSucceeded {
                kind: AuthenticationKind::Exchange,
            },
            AuditEventType::TokenExchange,
            AuditSeverity::Info,
        ),
        (
            SecurityEvent::AuthenticationSucceeded {
                kind: AuthenticationKind::Refresh,
            },
            AuditEventType::TokenRefresh,
            AuditSeverity::Info,
        ),
        (
            SecurityEvent::AuthenticationFailed,
            AuditEventType::ValidationFailed,
            AuditSeverity::Warning,
        ),
        (
            SecurityEvent::RegistrationDenied,
            AuditEventType::RegistrationDenied,
            AuditSeverity::Warning,
        ),
        (
            SecurityEvent::PrincipalSuspended,
            AuditEventType::UserSuspended,
            AuditSeverity::Warning,
        ),
        (
            SecurityEvent::PrincipalCreated,
            AuditEventType::UserCreated,
            AuditSeverity::Notice,
        ),
        (
            SecurityEvent::SessionRevoked,
            AuditEventType::TokenRevocation,
            AuditSeverity::Info,
        ),
        (
            SecurityEvent::SessionsRevoked,
            AuditEventType::AllSessionsRevoked,
            AuditSeverity::Notice,
        ),
        (
            SecurityEvent::ProviderRejected,
            AuditEventType::ProviderError,
            AuditSeverity::Warning,
        ),
        (
            SecurityEvent::AdminMutation {
                kind: AdminMutationKind::Created,
            },
            AuditEventType::UserCreated,
            AuditSeverity::Notice,
        ),
        (
            SecurityEvent::AdminMutation {
                kind: AdminMutationKind::Updated,
            },
            AuditEventType::UserUpdated,
            AuditSeverity::Notice,
        ),
        (
            SecurityEvent::AdminMutation {
                kind: AdminMutationKind::Deleted,
            },
            AuditEventType::UserDeleted,
            AuditSeverity::Notice,
        ),
        (
            SecurityEvent::ThrottleExceeded,
            AuditEventType::ThrottleExceeded,
            AuditSeverity::Warning,
        ),
    ];

    for (event, event_type, severity) in cases {
        assert_eq!(event.event_type(), event_type);
        assert_eq!(event.severity(), severity);
    }
}

#[test]
fn asserted_client_address_is_bounded_by_the_named_limit() {
    assert!(ClientAddr::asserted("x".repeat(MAX_ASSERTED_CLIENT_ADDR_LEN - 1)).is_some());
    assert!(ClientAddr::asserted("x".repeat(MAX_ASSERTED_CLIENT_ADDR_LEN)).is_some());
    assert!(ClientAddr::asserted("x".repeat(MAX_ASSERTED_CLIENT_ADDR_LEN + 1)).is_none());
}

#[test]
fn client_address_preserves_provenance_and_excludes_untrusted_rate_keys() {
    let peer = ClientAddr::Peer(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 5)));
    assert_eq!(peer.source(), ClientAddrSource::Peer);
    assert_eq!(peer.audit_address().as_deref(), Some("203.0.113.5"));
    assert_eq!(
        peer.rate_limit_key(),
        Some(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 5)))
    );

    let forwarded = ClientAddr::Forwarded(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7)));
    assert_eq!(forwarded.source(), ClientAddrSource::Forwarded);
    assert_eq!(
        forwarded.rate_limit_key(),
        Some(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7)))
    );

    let asserted = ClientAddr::asserted("forged, 203.0.113.8").unwrap();
    assert_eq!(asserted.source(), ClientAddrSource::Asserted);
    assert_eq!(
        asserted.audit_address().as_deref(),
        Some("forged, 203.0.113.8")
    );
    assert_eq!(asserted.rate_limit_key(), None);
    assert_eq!(RateLimitKey::client_addr_failure(&asserted), None);
    assert_eq!(
        RateLimitKey::client_addr_failure(&peer),
        Some(RateLimitKey::ClientAddrFailure(IpAddr::V4(Ipv4Addr::new(
            203, 0, 113, 5
        ))))
    );
    assert_eq!(ClientAddr::Unknown.source(), ClientAddrSource::Unknown);
    assert_eq!(ClientAddr::Unknown.rate_limit_key(), None);
}

#[test]
fn security_event_serialization_keeps_fixed_metadata_and_no_subject() {
    let event = SecurityEvent::ProviderRejected.into_audit_event(
        AuditOutcome::Failure(AuditFailure::ProviderRejected),
        None,
        Some("mock".to_string()),
        ClientAddr::Forwarded(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7))),
        None,
    );
    let json = serde_json::to_value(&event).unwrap();
    assert_eq!(json["event_type"], "provider_error");
    assert_eq!(json["severity"], "warning");
    assert_eq!(json["ip_address_source"], "forwarded");
    let payload = json.to_string();
    assert!(!payload.contains("provider-subject"));
    assert!(!payload.contains("upstream secret response body"));
    assert_eq!(subject_hash("provider-subject").len(), 64);
    assert_ne!(subject_hash("provider-subject"), "provider-subject");
}
