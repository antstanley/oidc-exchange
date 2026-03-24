use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    /// ULID
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub severity: AuditSeverity,
    pub event_type: AuditEventType,
    /// User ID if known
    pub actor: Option<String>,
    pub provider: Option<String>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub detail: HashMap<String, Value>,
    pub outcome: AuditOutcome,
}

/// Mapped to syslog severity levels (RFC 5424)
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuditSeverity {
    Emergency = 0,
    Alert = 1,
    Critical = 2,
    Error = 3,
    Warning = 4,
    Notice = 5,
    Info = 6,
    Debug = 7,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuditEventType {
    TokenExchange,
    TokenRefresh,
    TokenRevocation,
    SessionRevoked,
    AllSessionsRevoked,
    UserCreated,
    UserUpdated,
    UserSuspended,
    UserDeleted,
    ValidationFailed,
    RegistrationDenied,
    ProviderError,
    Unauthorized,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditOutcome {
    Success,
    Failure { reason: String },
}

impl Serialize for AuditOutcome {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap;

        match self {
            AuditOutcome::Success => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("status", "success")?;
                map.end()
            }
            AuditOutcome::Failure { reason } => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("status", "failure")?;
                map.serialize_entry("reason", reason)?;
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for AuditOutcome {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::{self, MapAccess, Visitor};
        use std::fmt;

        struct AuditOutcomeVisitor;

        impl<'de> Visitor<'de> for AuditOutcomeVisitor {
            type Value = AuditOutcome;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a map with 'status' key")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut status: Option<String> = None;
                let mut reason: Option<String> = None;

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "status" => {
                            status = Some(map.next_value()?);
                        }
                        "reason" => {
                            reason = Some(map.next_value()?);
                        }
                        _ => {
                            let _ = map.next_value::<serde::de::IgnoredAny>()?;
                        }
                    }
                }

                let status = status.ok_or_else(|| de::Error::missing_field("status"))?;
                match status.as_str() {
                    "success" => Ok(AuditOutcome::Success),
                    "failure" => {
                        let reason =
                            reason.ok_or_else(|| de::Error::missing_field("reason"))?;
                        Ok(AuditOutcome::Failure { reason })
                    }
                    other => Err(de::Error::unknown_variant(
                        other,
                        &["success", "failure"],
                    )),
                }
            }
        }

        deserializer.deserialize_map(AuditOutcomeVisitor)
    }
}
