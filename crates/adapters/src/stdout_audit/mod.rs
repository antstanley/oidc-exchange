//! Stdout/stderr audit log adapter.
//!
//! Emits audit events as structured JSON to stdout or stderr. Useful for
//! container deployments where logs are collected by the orchestrator
//! (CloudWatch, Datadog, Fluentd, etc.) and for local development.

use async_trait::async_trait;
use oidc_exchange_core::domain::{AuditEvent, AuditSeverity};
use oidc_exchange_core::error::{Error, Result};
use oidc_exchange_core::ports::AuditLog;

/// Output target for the stdout audit adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputTarget {
    /// Write all events to stdout.
    Stdout,
    /// Write all events to stderr.
    Stderr,
    /// Route by severity: Error and above → stderr, everything else → stdout.
    Auto,
}

/// Audit log implementation that writes structured JSON events to stdout/stderr.
pub struct StdoutAuditLog {
    target: OutputTarget,
}

impl StdoutAuditLog {
    pub fn new(target: OutputTarget) -> Self {
        Self { target }
    }
}

#[async_trait]
impl AuditLog for StdoutAuditLog {
    async fn emit(&self, event: &AuditEvent) -> Result<()> {
        let json = serde_json::to_string(event).map_err(|e| Error::AuditError {
            detail: format!("failed to serialize audit event: {e}"),
        })?;

        let use_stderr = match self.target {
            OutputTarget::Stdout => false,
            OutputTarget::Stderr => true,
            OutputTarget::Auto => (event.severity as u8) <= (AuditSeverity::Error as u8),
        };

        if use_stderr {
            eprintln!("{json}");
        } else {
            println!("{json}");
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use oidc_exchange_core::domain::{AuditEventType, AuditOutcome};
    use std::collections::HashMap;

    fn sample_event(severity: AuditSeverity) -> AuditEvent {
        AuditEvent {
            id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".to_string(),
            timestamp: Utc::now(),
            severity,
            event_type: AuditEventType::TokenExchange,
            actor: Some("usr_abc123".to_string()),
            provider: Some("google".to_string()),
            ip_address: Some("10.0.0.1".to_string()),
            user_agent: Some("test-agent/1.0".to_string()),
            detail: {
                let mut m = HashMap::new();
                m.insert(
                    "grant_type".to_string(),
                    serde_json::Value::String("authorization_code".to_string()),
                );
                m
            },
            outcome: AuditOutcome::Success,
        }
    }

    #[tokio::test]
    async fn emit_stdout_succeeds() {
        let adapter = StdoutAuditLog::new(OutputTarget::Stdout);
        let event = sample_event(AuditSeverity::Info);
        adapter.emit(&event).await.expect("emit should succeed");
    }

    #[tokio::test]
    async fn emit_stderr_succeeds() {
        let adapter = StdoutAuditLog::new(OutputTarget::Stderr);
        let event = sample_event(AuditSeverity::Error);
        adapter.emit(&event).await.expect("emit should succeed");
    }

    #[tokio::test]
    async fn emit_auto_routes_by_severity() {
        let adapter = StdoutAuditLog::new(OutputTarget::Auto);

        // Info → stdout (no error)
        let info_event = sample_event(AuditSeverity::Info);
        adapter
            .emit(&info_event)
            .await
            .expect("info emit should succeed");

        // Error → stderr (no error)
        let error_event = sample_event(AuditSeverity::Error);
        adapter
            .emit(&error_event)
            .await
            .expect("error emit should succeed");
    }
}
