//! Operator-principal domain types for the admin plane.
//!
//! The transport-level primitives the admin plane throttles and audits with —
//! [`crate::domain::ClientAddr`], [`crate::domain::RateLimitKey`],
//! [`crate::domain::RateLimitDecision`], [`crate::domain::SecurityEvent`] —
//! are owned by [`crate::domain::audit`] (the
//! `audit_and_throttle_authentication_failures` change); this module carries
//! only the operator-specific vocabulary layered on top of them. The admin
//! listener sits behind no untrusted proxy, so its callers key the
//! `OperatorAuth` budget from `ClientAddr::Peer` alone — never a forwarded or
//! client-asserted value.

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

    /// Check the type's invariants: the id is non-empty, and the
    /// shared-secret mechanism pairs only with [`UNATTRIBUTED_OPERATOR_ID`] —
    /// that mechanism proves possession of a string and identifies nobody, so
    /// a "shared_secret" event naming a principal would corrupt the very
    /// distinction the reserved id exists to preserve.
    ///
    /// Called wherever a principal enters an audit event, so an ill-formed
    /// principal crashes loudly instead of being recorded.
    pub fn assert_invariants(&self) {
        assert!(
            !self.id.is_empty(),
            "operator principal id must be non-empty"
        );
        assert_eq!(
            self.mechanism != OperatorAuthMechanism::SharedSecret,
            self.id != UNATTRIBUTED_OPERATOR_ID,
            "the unattributed id belongs to the shared-secret mechanism alone, and \
             that mechanism must never claim any other id"
        );
    }
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
    /// The fixed diagnostic string for this reason, for structured log fields;
    /// the durable audit channel uses [`Self::audit_failure`] instead.
    pub fn as_str(&self) -> &'static str {
        match self {
            OperatorAuthFailureReason::MissingCredential => "missing_credential",
            OperatorAuthFailureReason::InvalidCredential => "invalid_credential",
            OperatorAuthFailureReason::NotConfigured => "not_configured",
        }
    }

    /// The closed [`crate::domain::AuditFailure`] this reason renders to on
    /// the mandatory audit channel — typed, never free-form text, so audit
    /// queries can rely on exact matching and the presented credential is
    /// never among the reasons.
    pub fn audit_failure(self) -> crate::domain::AuditFailure {
        match self {
            OperatorAuthFailureReason::MissingCredential => {
                crate::domain::AuditFailure::MissingCredential
            }
            OperatorAuthFailureReason::InvalidCredential => {
                crate::domain::AuditFailure::InvalidCredential
            }
            OperatorAuthFailureReason::NotConfigured => {
                crate::domain::AuditFailure::NotConfigured
            }
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

    /// The audit reason vocabulary is the spec's three fixed, typed
    /// `AuditFailure` variants (serialized snake_case) — no free-form reason
    /// text may enter the channel.
    #[test]
    fn failure_reasons_use_the_fixed_vocabulary() {
        let cases = [
            (OperatorAuthFailureReason::MissingCredential, "\"missing_credential\""),
            (OperatorAuthFailureReason::InvalidCredential, "\"invalid_credential\""),
            (OperatorAuthFailureReason::NotConfigured, "\"not_configured\""),
        ];
        for (reason, wire) in cases {
            let json = serde_json::to_string(&reason.audit_failure())
                .expect("audit failure serializes");
            assert_eq!(json, wire);
        }
    }

    /// Invariant checks accept the two well-formed shapes: a named principal
    /// from a named mechanism and the reserved unattributed pair.
    #[test]
    fn invariants_accept_well_formed_principals() {
        let token = OperatorPrincipal {
            id: "usr_operator_alice".to_string(),
            mechanism: OperatorAuthMechanism::OperatorToken,
        };
        token.assert_invariants();

        let mtls = OperatorPrincipal {
            id: "CN=ops.example.com".to_string(),
            mechanism: OperatorAuthMechanism::MutualTls,
        };
        mtls.assert_invariants();

        OperatorPrincipal::unattributed().assert_invariants();
    }

    /// Negative space: a shared-secret principal naming a person, a named
    /// mechanism claiming the reserved literal, or an empty id are all
    /// programmer errors that must crash at the attribution boundary.
    #[test]
    #[should_panic(expected = "must never claim any other id")]
    fn invariants_reject_shared_secret_claiming_a_named_id() {
        let forged = OperatorPrincipal {
            id: "usr_operator_alice".to_string(),
            mechanism: OperatorAuthMechanism::SharedSecret,
        };
        forged.assert_invariants();
    }

    #[test]
    #[should_panic(expected = "belongs to the shared-secret mechanism alone")]
    fn invariants_reject_named_mechanism_claiming_the_reserved_id() {
        let forged = OperatorPrincipal {
            id: UNATTRIBUTED_OPERATOR_ID.to_string(),
            mechanism: OperatorAuthMechanism::OperatorToken,
        };
        forged.assert_invariants();
    }

    #[test]
    #[should_panic(expected = "id must be non-empty")]
    fn invariants_reject_an_empty_principal_id() {
        let empty = OperatorPrincipal {
            id: String::new(),
            mechanism: OperatorAuthMechanism::OperatorToken,
        };
        empty.assert_invariants();
    }
}
