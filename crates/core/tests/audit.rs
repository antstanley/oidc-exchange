use std::collections::HashMap;

use oidc_exchange_core::config::{AppConfig, AuditConfig};
use oidc_exchange_core::domain::{AuditEventType, AuditOutcome, AuditSeverity};
use oidc_exchange_core::ports::IdentityProvider;
use oidc_exchange_core::service::{create_audit_event, AppService};

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
        Box::new(MockRateLimiter::new()),
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
        None,
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
        None,
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
        AuditOutcome::Failure {
            reason: "something went wrong".to_string(),
        },
        Some("user-1".to_string()),
        Some("mock".to_string()),
        None,
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
        Some("203.0.113.5".to_string()),
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
        None,
        None,
    );

    assert_eq!(event.ip_address, None);
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
        AuditOutcome::Failure {
            reason: "unknown token".to_string(),
        },
        None,
        Some("mock".to_string()),
        None,
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
        None,
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
        AuditOutcome::Failure {
            reason: "unknown token".to_string(),
        },
        None,
        Some("mock".to_string()),
        None,
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
