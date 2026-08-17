use std::collections::HashMap;
use std::net::IpAddr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Maximum length of provider identifiers retained in rate-limit keys.
pub const MAX_RATE_LIMIT_PROVIDER_LEN: usize = 128;
/// Length of a SHA-256 hexadecimal digest used for subject rate-limit keys.
pub const SUBJECT_HASH_HEX_LEN: usize = 64;
/// Maximum length of a client-authored address retained in audit records.
pub const MAX_ASSERTED_CLIENT_ADDR_LEN: usize = 128;

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
    pub ip_address_source: ClientAddrSource,
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
    ThrottleExceeded,
}

/// The server's confidence in the address recorded on an audit event.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClientAddrSource {
    Peer,
    Forwarded,
    Asserted,
    Unknown,
}

/// A client address together with how the service learned it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientAddr {
    /// Observed directly by the server or supplied by the runtime platform.
    Peer(IpAddr),
    /// Read from a forwarded header only after a trusted proxy was established.
    Forwarded(IpAddr),
    /// Client-authored, length-bounded, and never eligible for rate limiting.
    Asserted(AssertedClientAddr),
    /// No usable address was available.
    Unknown,
}

/// A client-authored address that is safe to retain in audit records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssertedClientAddr(String);

impl AssertedClientAddr {
    /// Validates and retains an asserted address only when it fits the audit bound.
    pub fn new(value: impl Into<String>) -> Option<Self> {
        let value = value.into();
        (value.len() <= MAX_ASSERTED_CLIENT_ADDR_LEN).then_some(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl ClientAddr {
    /// Builds a bounded client-authored address for audit-only use.
    pub fn asserted(value: impl Into<String>) -> Option<Self> {
        AssertedClientAddr::new(value).map(Self::Asserted)
    }

    pub fn source(&self) -> ClientAddrSource {
        match self {
            Self::Peer(_) => ClientAddrSource::Peer,
            Self::Forwarded(_) => ClientAddrSource::Forwarded,
            Self::Asserted(_) => ClientAddrSource::Asserted,
            Self::Unknown => ClientAddrSource::Unknown,
        }
    }

    pub fn audit_address(&self) -> Option<String> {
        match self {
            Self::Peer(address) | Self::Forwarded(address) => Some(address.to_string()),
            Self::Asserted(value) => Some(value.as_str().to_owned()),
            Self::Unknown => None,
        }
    }

    /// Returns an address that the server established and may safely use as a rate key.
    pub fn rate_limit_key(&self) -> Option<IpAddr> {
        match self {
            Self::Peer(address) | Self::Forwarded(address) => Some(*address),
            Self::Asserted(_) | Self::Unknown => None,
        }
    }
}

/// The authentication and authorization outcomes that always carry fixed audit metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityEvent {
    AuthenticationSucceeded { kind: AuthenticationKind },
    AuthenticationFailed,
    RegistrationDenied,
    PrincipalSuspended,
    PrincipalCreated,
    SessionRevoked,
    SessionsRevoked,
    ProviderRejected,
    AdminMutation { kind: AdminMutationKind },
    ThrottleExceeded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthenticationKind {
    Exchange,
    Refresh,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminMutationKind {
    Created,
    Updated,
    Suspended,
    Deleted,
}

impl SecurityEvent {
    pub fn severity(self) -> AuditSeverity {
        match self {
            Self::AuthenticationSucceeded { .. } | Self::SessionRevoked => AuditSeverity::Info,
            Self::AuthenticationFailed
            | Self::RegistrationDenied
            | Self::PrincipalSuspended
            | Self::ProviderRejected
            | Self::ThrottleExceeded => AuditSeverity::Warning,
            Self::PrincipalCreated | Self::SessionsRevoked | Self::AdminMutation { .. } => {
                AuditSeverity::Notice
            }
        }
    }

    pub fn event_type(self) -> AuditEventType {
        match self {
            Self::AuthenticationSucceeded {
                kind: AuthenticationKind::Exchange,
            } => AuditEventType::TokenExchange,
            Self::AuthenticationSucceeded {
                kind: AuthenticationKind::Refresh,
            } => AuditEventType::TokenRefresh,
            Self::AuthenticationFailed => AuditEventType::ValidationFailed,
            Self::RegistrationDenied => AuditEventType::RegistrationDenied,
            Self::PrincipalSuspended => AuditEventType::UserSuspended,
            Self::PrincipalCreated => AuditEventType::UserCreated,
            Self::SessionRevoked => AuditEventType::TokenRevocation,
            Self::SessionsRevoked => AuditEventType::AllSessionsRevoked,
            Self::ProviderRejected => AuditEventType::ProviderError,
            Self::AdminMutation {
                kind: AdminMutationKind::Created,
            } => AuditEventType::UserCreated,
            Self::AdminMutation {
                kind: AdminMutationKind::Updated,
            } => AuditEventType::UserUpdated,
            Self::AdminMutation {
                kind: AdminMutationKind::Suspended,
            } => AuditEventType::UserSuspended,
            Self::AdminMutation {
                kind: AdminMutationKind::Deleted,
            } => AuditEventType::UserDeleted,
            Self::ThrottleExceeded => AuditEventType::ThrottleExceeded,
        }
    }

    /// Builds an audit event with classifications fixed by this closed enum.
    #[allow(clippy::too_many_arguments)]
    pub fn into_audit_event(
        self,
        outcome: AuditOutcome,
        actor: Option<String>,
        provider: Option<String>,
        client_addr: ClientAddr,
        user_agent: Option<String>,
    ) -> AuditEvent {
        AuditEvent {
            id: ulid::Ulid::new().to_string(),
            timestamp: Utc::now(),
            severity: self.severity(),
            event_type: self.event_type(),
            actor,
            provider,
            ip_address: client_addr.audit_address(),
            ip_address_source: client_addr.source(),
            user_agent,
            detail: HashMap::new(),
            outcome,
        }
    }
}

/// SHA-256 hex digest of an upstream subject suitable for durable audit metadata.
pub fn subject_hash(subject: &str) -> String {
    hex::encode(Sha256::digest(subject.as_bytes()))
}

/// A bounded, non-raw identity for one rate-limit bucket.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RateLimitKey {
    ClientAddr(IpAddr),
    /// A separate bucket for failed authentication attempts from a trusted client address.
    ClientAddrFailure(IpAddr),
    Subject {
        provider: Option<String>,
        subject_hash: String,
    },
    Provider(String),
}

impl RateLimitKey {
    /// Builds a failed-authentication bucket from a server-established client address.
    pub fn client_addr_failure(client_addr: &ClientAddr) -> Option<Self> {
        client_addr.rate_limit_key().map(Self::ClientAddrFailure)
    }

    /// Builds a provider bucket only when the identifier is bounded.
    pub fn provider(provider: impl Into<String>) -> Option<Self> {
        let provider = provider.into();
        (provider.len() <= MAX_RATE_LIMIT_PROVIDER_LEN).then_some(Self::Provider(provider))
    }

    /// Builds a subject bucket from its raw subject without retaining that value.
    pub fn subject(provider: Option<&str>, subject: &str) -> Option<Self> {
        let provider = match provider {
            Some(provider) if provider.len() > MAX_RATE_LIMIT_PROVIDER_LEN => return None,
            Some(provider) => Some(provider.to_owned()),
            None => None,
        };
        Some(Self::Subject {
            provider,
            subject_hash: subject_hash(subject),
        })
    }

    /// Validates an externally supplied subject digest without accepting raw subjects.
    pub fn subject_hashed(provider: Option<String>, subject_hash: String) -> Option<Self> {
        if provider
            .as_ref()
            .is_some_and(|provider| provider.len() > MAX_RATE_LIMIT_PROVIDER_LEN)
            || subject_hash.len() != SUBJECT_HASH_HEX_LEN
            || !subject_hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return None;
        }
        Some(Self::Subject {
            provider,
            subject_hash,
        })
    }
}

/// The result of consuming one unit from a rate-limit bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateLimitDecision {
    Allow,
    Deny { retry_after_secs: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditFailure {
    AuthenticationFailed,
    RegistrationDenied,
    PrincipalSuspended,
    ProviderRejected,
    ThrottleExceeded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditOutcome {
    Success,
    Failure(AuditFailure),
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
            AuditOutcome::Failure(reason) => {
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
                let mut reason: Option<AuditFailure> = None;

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
                        Ok(AuditOutcome::Failure(reason))
                    }
                    other => Err(de::Error::unknown_variant(other, &["success", "failure"])),
                }
            }
        }

        deserializer.deserialize_map(AuditOutcomeVisitor)
    }
}
