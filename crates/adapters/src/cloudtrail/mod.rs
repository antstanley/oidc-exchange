use async_trait::async_trait;
use aws_sdk_cloudtraildata::types::AuditEvent as CloudTrailEvent;
use oidc_exchange_core::domain::AuditEvent;
use oidc_exchange_core::error::{Error, Result};
use oidc_exchange_core::ports::AuditLog;

/// Sends audit events to AWS CloudTrail Lake via the `PutAuditEvents` API.
pub struct CloudTrailAuditLog {
    client: aws_sdk_cloudtraildata::Client,
    channel_arn: String,
}

impl CloudTrailAuditLog {
    pub fn new(client: aws_sdk_cloudtraildata::Client, channel_arn: String) -> Self {
        Self {
            client,
            channel_arn,
        }
    }
}

/// Convert our domain `AuditEvent` into a CloudTrail Lake `AuditEvent` struct.
fn to_cloudtrail_event(event: &AuditEvent) -> Result<CloudTrailEvent> {
    let event_data = serde_json::to_string(event).map_err(|e| Error::AuditError {
        detail: format!("failed to serialize audit event: {e}"),
    })?;

    CloudTrailEvent::builder()
        .id(&event.id)
        .event_data(event_data)
        .build()
        .map_err(|e| Error::AuditError {
            detail: format!("failed to build CloudTrail event: {e}"),
        })
}

#[async_trait]
impl AuditLog for CloudTrailAuditLog {
    async fn emit(&self, event: &AuditEvent) -> Result<()> {
        let ct_event = to_cloudtrail_event(event)?;

        self.client
            .put_audit_events()
            .channel_arn(&self.channel_arn)
            .audit_events(ct_event)
            .send()
            .await
            .map_err(|e| Error::AuditError {
                detail: format!("CloudTrail PutAuditEvents failed: {e}"),
            })?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use oidc_exchange_core::domain::{AuditEventType, AuditOutcome, AuditSeverity};
    use std::collections::HashMap;

    fn sample_event() -> AuditEvent {
        AuditEvent {
            id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".to_string(),
            timestamp: Utc::now(),
            severity: AuditSeverity::Info,
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

    #[test]
    fn test_to_cloudtrail_event_format() {
        let event = sample_event();
        let ct_event = to_cloudtrail_event(&event).expect("conversion should succeed");

        // The CloudTrail event ID should match our audit event ID
        assert_eq!(ct_event.id(), event.id.as_str());

        // The event_data should be valid JSON containing our fields
        let event_data = ct_event.event_data();
        let parsed: serde_json::Value =
            serde_json::from_str(event_data).expect("event_data should be valid JSON");

        assert_eq!(parsed["id"], "01ARZ3NDEKTSV4RRFFQ69G5FAV");
        assert_eq!(parsed["severity"], "info");
        assert_eq!(parsed["event_type"], "token_exchange");
        assert_eq!(parsed["actor"], "usr_abc123");
        assert_eq!(parsed["provider"], "google");
        assert_eq!(parsed["ip_address"], "10.0.0.1");
        assert_eq!(parsed["detail"]["grant_type"], "authorization_code");

        // outcome is a map with status
        assert_eq!(parsed["outcome"]["status"], "success");
    }

    #[test]
    fn test_to_cloudtrail_event_failure_outcome() {
        let mut event = sample_event();
        event.outcome = AuditOutcome::Failure {
            reason: "invalid_grant".to_string(),
        };

        let ct_event = to_cloudtrail_event(&event).expect("conversion should succeed");
        let parsed: serde_json::Value =
            serde_json::from_str(ct_event.event_data()).expect("valid JSON");

        assert_eq!(parsed["outcome"]["status"], "failure");
        assert_eq!(parsed["outcome"]["reason"], "invalid_grant");
    }
}
