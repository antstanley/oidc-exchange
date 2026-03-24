use std::collections::HashMap;

use oidc_exchange_core::config::{AppConfig, AuditConfig};
use oidc_exchange_core::domain::{AuditEventType, AuditOutcome, AuditSeverity};
use oidc_exchange_core::ports::IdentityProvider;
use oidc_exchange_core::service::{create_audit_event, AppService};

use oidc_exchange_test_utils::{
    MockAuditLog, MockIdentityProvider, MockKeyManager, MockRepository, MockUserSync,
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

fn make_service_with_audit(audit: MockAuditLog, config: AppConfig) -> AppService {
    let provider = MockIdentityProvider::new("mock");
    let provider_id = provider.provider_id().to_string();
    let mut providers: HashMap<String, Box<dyn IdentityProvider>> = HashMap::new();
    providers.insert(provider_id, Box::new(provider));

    AppService::new(
        Box::new(MockRepository::new()),
        Box::new(MockKeyManager::new()),
        Box::new(audit),
        Box::new(MockUserSync::new()),
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
    );

    let result = svc.emit_audit(event).await;
    assert!(result.is_ok(), "successful audit emit should return Ok");

    let events = audit_clone.events().await;
    assert_eq!(events.len(), 1, "one event should have been recorded");
    assert_eq!(events[0].event_type, AuditEventType::TokenExchange);
    assert_eq!(events[0].severity, AuditSeverity::Info);
    assert_eq!(events[0].actor.as_deref(), Some("user-1"));
    assert_eq!(events[0].provider.as_deref(), Some("mock"));
}
