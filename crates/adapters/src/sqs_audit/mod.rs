//! SQS-based audit log adapter.
//!
//! Sends audit events as JSON messages to an SQS queue. The intended downstream
//! pipeline is:
//!
//! SQS → Amazon Data Firehose → S3 Tables (Apache Iceberg format, Parquet files)
//!
//! This provides a durable, queryable audit trail with low-latency ingestion and
//! columnar storage optimized for analytical queries via Athena or Spark.

use async_trait::async_trait;
use aws_sdk_sqs::types::MessageAttributeValue;
use oidc_exchange_core::domain::AuditEvent;
use oidc_exchange_core::error::{Error, Result};
use oidc_exchange_core::ports::AuditLog;
use tracing::instrument;

/// Audit log implementation that sends events to an Amazon SQS queue.
pub struct SqsAuditLog {
    client: aws_sdk_sqs::Client,
    queue_url: String,
}

impl SqsAuditLog {
    pub fn new(client: aws_sdk_sqs::Client, queue_url: String) -> Self {
        Self { client, queue_url }
    }
}

#[async_trait]
impl AuditLog for SqsAuditLog {
    #[instrument(skip(self, event), fields(event_id = %event.id))]
    async fn emit(&self, event: &AuditEvent) -> Result<()> {
        let json_str = serde_json::to_string(event).map_err(|e| Error::AuditError {
            detail: format!("failed to serialize audit event: {e}"),
        })?;

        let severity_str = serde_json::to_string(&event.severity)
            .map_err(|e| Error::AuditError {
                detail: format!("failed to serialize severity: {e}"),
            })?
            .trim_matches('"')
            .to_string();

        let severity_attr = MessageAttributeValue::builder()
            .data_type("String")
            .string_value(severity_str)
            .build()
            .map_err(|e| Error::AuditError {
                detail: e.to_string(),
            })?;

        let mut req = self
            .client
            .send_message()
            .queue_url(&self.queue_url)
            .message_body(json_str)
            .message_attributes("severity", severity_attr);

        if self.queue_url.contains(".fifo") {
            let event_type_str = serde_json::to_string(&event.event_type)
                .map_err(|e| Error::AuditError {
                    detail: format!("failed to serialize event_type: {e}"),
                })?
                .trim_matches('"')
                .to_string();

            req = req
                .message_deduplication_id(&event.id)
                .message_group_id(event_type_str);
        }

        req.send().await.map_err(|e| Error::AuditError {
            detail: e.to_string(),
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
    fn test_fifo_detection() {
        let url_fifo = "https://sqs.us-east-1.amazonaws.com/123456789012/audit-events.fifo";
        let url_standard = "https://sqs.us-east-1.amazonaws.com/123456789012/audit-events";

        assert!(url_fifo.contains(".fifo"));
        assert!(!url_standard.contains(".fifo"));
    }

    #[test]
    fn test_event_serializes_to_json() {
        let event = sample_event();
        let json_str = serde_json::to_string(&event).expect("should serialize");
        let parsed: serde_json::Value =
            serde_json::from_str(&json_str).expect("should be valid JSON");

        assert_eq!(parsed["id"], "01ARZ3NDEKTSV4RRFFQ69G5FAV");
        assert_eq!(parsed["severity"], "info");
        assert_eq!(parsed["event_type"], "token_exchange");
    }
}
