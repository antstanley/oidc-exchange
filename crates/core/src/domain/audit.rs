use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::domain::OperatorPrincipal;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    /// ULID
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub severity: AuditSeverity,
    pub event_type: AuditEventType,
    /// User id if known — the subject of the action.
    pub actor: Option<String>,
    /// The operator principal that performed the action. Present on
    /// `/internal/*` operations (under the shared-secret compatibility
    /// mechanism it is present and explicitly `unattributed`), `None` on the
    /// exchange plane, where there is no operator.
    pub operator: Option<OperatorPrincipal>,
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
    /// VENDORED SEAM (task 03): variant added by sibling PR #24's audit-type
    /// changes; carried here so the admin plane can record throttle lockouts.
    /// At merge time this arm is deleted in favour of #24's identical addition.
    ThrottleExceeded,
}

/// The fixed failure-reason vocabulary for mandatory-channel security events
/// (`AuditOutcome::Failure { reason }`). Reasons on this channel are closed
/// constants, never free-form text, so audit queries can rely on exact
/// matching; the presented credential is never among them.
///
/// VENDORED SEAM (task 03): mirrors PR #24's typed failure-reason design in a
/// shape compatible with this branch's string-carrying `AuditOutcome`; PR
/// #24's typed `AuditFailure` enum replaces these constants at merge time.
pub mod security_failure_reasons {
    /// A rejected operator authentication: no credential presented.
    pub const MISSING_CREDENTIAL: &str = "missing_credential";
    /// A rejected operator authentication: credential failed verification.
    pub const INVALID_CREDENTIAL: &str = "invalid_credential";
    /// A rejected operator authentication: no mechanism usable/configured.
    pub const NOT_CONFIGURED: &str = "not_configured";
    /// The `OperatorAuth` rate-limit budget is exhausted (the lockout).
    pub const THROTTLE_EXCEEDED: &str = "throttle_exceeded";
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
                        let reason = reason.ok_or_else(|| de::Error::missing_field("reason"))?;
                        Ok(AuditOutcome::Failure { reason })
                    }
                    other => Err(de::Error::unknown_variant(other, &["success", "failure"])),
                }
            }
        }

        deserializer.deserialize_map(AuditOutcomeVisitor)
    }
}
