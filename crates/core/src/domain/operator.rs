//! Vendored seam + operator-principal domain types for the admin plane.
//!
//! `VENDORED SEAM (task 03)` — [`ClientAddr`], [`RateLimitKey`],
//! [`RateLimitDecision`] are minimal vendored primitives from sibling PR #24
//! (`2026-08-05-audit_and_throttle_authentication_failures`, branch
//! `spec/audit-and-throttle-auth-failures`). This branch predates that PR; at
//! merge time these definitions are deleted in favour of #24's canonical ones
//! and call sites are re-pointed. Only the subset task 05 needs is carried:
//! peer-only provenance (the admin listener sits behind no untrusted proxy, so
//! #24's `Forwarded`/`Asserted` variants are intentionally not vendored) plus
//! the admin plane's own limiter key, which #24 does not define.

use std::net::IpAddr;

use serde::{Deserialize, Serialize};

/// How an `/internal/*` request was authenticated. Wire values are the closed
/// set from the canonical types schema's `OperatorAuthMechanism`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperatorAuthMechanism {
    /// The schema spells this value `mtls`, not `mutual_tls`, so audit events
    /// and any future serialization agree with the published contract.
    #[serde(rename = "mtls")]
    MutualTls,
    OperatorToken,
    SharedSecret,
}

/// The reserved principal id recorded when a request authenticated via the
/// shared-secret compatibility mechanism: that mechanism proves possession of
/// a string and identifies nobody, so the event says so explicitly rather than
/// omitting identity.
pub const UNATTRIBUTED_OPERATOR_ID: &str = "unattributed";

/// The authenticated identity behind an `/internal/*` request.
///
/// Every admin service method receives one and every audit event an admin
/// operation emits carries it. [`OperatorAuthMechanism::SharedSecret`] always
/// pairs with [`UNATTRIBUTED_OPERATOR_ID`] — construct that pair through
/// [`OperatorPrincipal::unattributed`] so it cannot drift. No field here holds
/// credential material: the bearer secret/token itself never enters this type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperatorPrincipal {
    /// Certificate subject, operator-token `sub`, or [`UNATTRIBUTED_OPERATOR_ID`].
    pub id: String,
    pub mechanism: OperatorAuthMechanism,
}

impl OperatorPrincipal {
    /// The explicit shared-secret principal: present, but named as
    /// unattributed, so an audit reader can distinguish attributed actions
    /// from the compatibility path without inspecting configuration.
    pub fn unattributed() -> Self {
        Self {
            id: UNATTRIBUTED_OPERATOR_ID.to_string(),
            mechanism: OperatorAuthMechanism::SharedSecret,
        }
    }
}

/// A client address together with how the service learned it.
///
/// VENDORED SEAM (task 03): subset of PR #24's type. Only the peer variant is
/// carried because the only consumer on this branch is the admin plane, whose
/// throttle key must be the connection's real peer — never a forwarded or
/// client-asserted value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientAddr {
    /// Observed directly by the server from the socket.
    Peer(IpAddr),
    /// No usable address was available (e.g. an embedded runtime with no
    /// connect info).
    Unknown,
}

impl ClientAddr {
    /// Returns an address the server established and may safely use as a
    /// rate-limit key. `Unknown` yields nothing: no key means no budget is
    /// drawn down, which fails toward serving rather than toward lockout —
    /// acceptable only because runtimes without a socket peer also have no
    /// externally reachable guessing surface.
    pub fn rate_limit_key(&self) -> Option<IpAddr> {
        match self {
            ClientAddr::Peer(address) => Some(*address),
            ClientAddr::Unknown => None,
        }
    }

    /// The address to record on audit events, if any.
    pub fn audit_address(&self) -> Option<String> {
        match self {
            ClientAddr::Peer(address) => Some(address.to_string()),
            ClientAddr::Unknown => None,
        }
    }
}

/// A bounded, non-raw identity for one rate-limit bucket.
///
/// VENDORED SEAM (task 03): carries only this PR's `OperatorAuth` variant;
/// PR #24 defines the exchange-plane variants (`ClientAddr`, `Subject`,
/// `Provider`) and its merge folds both sets together.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RateLimitKey {
    /// Failed `/internal/*` operator authentications from one peer address.
    /// Kept distinct from any exchange-plane key so a burst of anonymous
    /// public traffic can never exhaust the operator budget and lock an
    /// administrator out of the plane they would use to respond to it.
    OperatorAuth(IpAddr),
}

/// The result of consuming one unit from a rate-limit bucket.
///
/// VENDORED SEAM (task 03): verbatim shape of PR #24's decision type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateLimitDecision {
    Allow,
    Deny { retry_after_secs: u64 },
}

/// The fixed failure reasons a rejected operator authentication can carry.
///
/// These are the audit stream's closed reason vocabulary for
/// `OperatorAuthenticationFailed`; the presented credential is never one of
/// them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperatorAuthFailureReason {
    /// No credential was presented where one was required.
    MissingCredential,
    /// A credential was presented and failed verification.
    InvalidCredential,
    /// The internal API has no usable authentication configured.
    NotConfigured,
}

impl OperatorAuthFailureReason {
    /// The fixed wire/audit string for this reason, drawn from the shared
    /// closed vocabulary in [`crate::domain::security_failure_reasons`] so
    /// every emitter spells the reasons identically.
    pub fn as_str(&self) -> &'static str {
        match self {
            OperatorAuthFailureReason::MissingCredential => {
                crate::domain::security_failure_reasons::MISSING_CREDENTIAL
            }
            OperatorAuthFailureReason::InvalidCredential => {
                crate::domain::security_failure_reasons::INVALID_CREDENTIAL
            }
            OperatorAuthFailureReason::NotConfigured => {
                crate::domain::security_failure_reasons::NOT_CONFIGURED
            }
        }
    }
}

/// The security outcomes that always emit through the mandatory audit channel,
/// bypassing every severity threshold.
///
/// VENDORED SEAM (task 03): reduced to the two admin-plane events this PR
/// constructs; PR #24 owns the full closed enum. Each maps onto an existing
/// (or sibling-added) `AuditEventType`:
/// - [`SecurityEvent::OperatorAuthenticationFailed`] renders as the
///   long-declared-but-unconstructed `AuditEventType::Unauthorized` at
///   warning severity.
/// - [`SecurityEvent::ThrottleExceeded`] renders as
///   `AuditEventType::ThrottleExceeded` (the variant PR #24 adds).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityEvent {
    OperatorAuthenticationFailed { reason: OperatorAuthFailureReason },
    ThrottleExceeded,
}

impl SecurityEvent {
    /// Severity is fixed per event kind, not caller-chosen.
    pub fn severity(self) -> crate::domain::AuditSeverity {
        // Both admin-plane security events warn: louder than routine notices,
        // below error-level paging.
        crate::domain::AuditSeverity::Warning
    }

    /// The durable audit classification this security event renders to.
    pub fn event_type(self) -> crate::domain::AuditEventType {
        match self {
            SecurityEvent::OperatorAuthenticationFailed { .. } => {
                crate::domain::AuditEventType::Unauthorized
            }
            SecurityEvent::ThrottleExceeded => crate::domain::AuditEventType::ThrottleExceeded,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shared-secret principal must be explicitly present yet explicitly
    /// unnamed: `id = unattributed` paired with the shared-secret mechanism,
    /// exactly the shape the canonical types schema describes.
    #[test]
    fn shared_secret_principal_is_present_but_unattributed() {
        let principal = OperatorPrincipal::unattributed();

        assert_eq!(principal.id, UNATTRIBUTED_OPERATOR_ID);
        assert_eq!(principal.id, "unattributed");
        assert_eq!(
            principal.mechanism,
            OperatorAuthMechanism::SharedSecret,
            "the unattributed id must never drift away from the shared-secret mechanism"
        );
    }

    /// The wire spelling of the mechanisms matches the schema's snake_case
    /// enum values exactly — a renamed variant would silently change every
    /// serialized audit event.
    #[test]
    fn mechanisms_serialize_to_the_schema_wire_values() {
        let cases = [
            (OperatorAuthMechanism::MutualTls, "\"mtls\""),
            (OperatorAuthMechanism::OperatorToken, "\"operator_token\""),
            (OperatorAuthMechanism::SharedSecret, "\"shared_secret\""),
        ];
        for (mechanism, wire) in cases {
            let json = serde_json::to_string(&mechanism).expect("mechanism serializes");
            assert_eq!(json, wire);
        }
    }

    /// Peer provenance yields a rate key and an audit address; unknown provenance
    /// yields neither — a missing address must never become a shared bucket.
    #[test]
    fn client_addr_provenance_controls_key_eligibility() {
        let peer = ClientAddr::Peer("192.0.2.10".parse().expect("valid ip"));
        assert_eq!(
            peer.rate_limit_key(),
            Some("192.0.2.10".parse().expect("valid ip"))
        );
        assert_eq!(peer.audit_address().as_deref(), Some("192.0.2.10"));

        let unknown = ClientAddr::Unknown;
        assert!(unknown.rate_limit_key().is_none());
        assert!(unknown.audit_address().is_none());
    }

    /// Security-event classifications are fixed: operator auth failures render
    /// as the previously-unconstructed Unauthorized audit type, throttle
    /// lockouts as ThrottleExceeded, both at warning severity.
    #[test]
    fn security_events_render_to_fixed_audit_classifications() {
        let failed = SecurityEvent::OperatorAuthenticationFailed {
            reason: OperatorAuthFailureReason::InvalidCredential,
        };
        assert_eq!(
            failed.event_type(),
            crate::domain::AuditEventType::Unauthorized
        );
        assert_eq!(failed.severity(), crate::domain::AuditSeverity::Warning);

        let throttled = SecurityEvent::ThrottleExceeded;
        assert_eq!(
            throttled.event_type(),
            crate::domain::AuditEventType::ThrottleExceeded
        );
        assert_eq!(throttled.severity(), crate::domain::AuditSeverity::Warning);
    }

    /// The audit reason vocabulary is the spec's three fixed strings — no
    /// free-form reason text may enter the channel.
    #[test]
    fn failure_reasons_use_the_fixed_vocabulary() {
        assert_eq!(
            OperatorAuthFailureReason::MissingCredential.as_str(),
            "missing_credential"
        );
        assert_eq!(
            OperatorAuthFailureReason::InvalidCredential.as_str(),
            "invalid_credential"
        );
        assert_eq!(
            OperatorAuthFailureReason::NotConfigured.as_str(),
            "not_configured"
        );
    }
}
