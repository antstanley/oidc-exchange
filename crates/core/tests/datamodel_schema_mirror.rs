//! S6: `schemas/datamodel.schema.json` mirrors the shipped audit enums. This
//! guard reads the committed schema and asserts its `AuditEvent.event_type` and
//! `outcome.reason` enum arrays equal the serde-rendered variant lists of
//! `AuditEventType` and `AuditFailure` — exactly, in order, not as a subset — so
//! the next enum addition fails this test (it is first a compile error in the
//! exhaustive `all_*` builders below, then an equality failure if the schema is
//! not updated) instead of drifting silently.

use oidc_exchange_core::domain::{AuditEventType, AuditFailure};

const SCHEMA: &str = include_str!("../../../schemas/datamodel.schema.json");

/// serde-render a unit enum variant to its wire string (snake_case), so the
/// schema is checked against serde's actual output rather than a hand-copied
/// list of strings.
fn rendered(value: impl serde::Serialize) -> String {
    serde_json::to_value(value)
        .expect("variant serializes")
        .as_str()
        .expect("a unit enum variant renders as a JSON string")
        .to_string()
}

/// Every `AuditEventType`, in declaration order. The match has no wildcard, so a
/// newly-added variant is a compile error here until it is listed.
fn all_event_types() -> Vec<AuditEventType> {
    use AuditEventType::*;
    let all = vec![
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
        RefreshTokenReuse,
        MissingCredential,
        InvalidCredential,
        NotConfigured,
    ];
    for variant in &all {
        match variant {
            TokenExchange | TokenRefresh | TokenRevocation | SessionRevoked
            | AllSessionsRevoked | UserCreated | UserUpdated | UserSuspended | UserDeleted
            | ValidationFailed | RegistrationDenied | ProviderError | Unauthorized
            | ThrottleExceeded | RefreshTokenReuse | MissingCredential | InvalidCredential
            | NotConfigured => {}
        }
    }
    all
}

/// Every `AuditFailure`, in declaration order (same wildcard-free guard).
fn all_failures() -> Vec<AuditFailure> {
    use AuditFailure::*;
    let all = vec![
        AuthenticationFailed,
        RegistrationDenied,
        PrincipalSuspended,
        ProviderRejected,
        ThrottleExceeded,
        RefreshTokenReuse,
        MissingCredential,
        InvalidCredential,
        NotConfigured,
    ];
    for variant in &all {
        match variant {
            AuthenticationFailed | RegistrationDenied | PrincipalSuspended | ProviderRejected
            | ThrottleExceeded | RefreshTokenReuse | MissingCredential | InvalidCredential
            | NotConfigured => {}
        }
    }
    all
}

fn schema() -> serde_json::Value {
    serde_json::from_str(SCHEMA).expect("datamodel schema is valid JSON")
}

#[test]
fn event_type_enum_mirrors_audit_event_type_variants() {
    let schema = schema();
    let schema_values: Vec<String> = schema["definitions"]["AuditEvent"]["properties"]
        ["event_type"]["enum"]
        .as_array()
        .expect("event_type enum array")
        .iter()
        .map(|value| value.as_str().expect("event_type is a string").to_string())
        .collect();
    let expected: Vec<String> = all_event_types().into_iter().map(rendered).collect();
    assert_eq!(
        schema_values, expected,
        "schemas/datamodel.schema.json event_type enum must equal the serde-rendered \
         AuditEventType variants exactly and in order"
    );
}

#[test]
fn outcome_reason_enum_mirrors_audit_failure_variants_plus_null() {
    let schema = schema();
    let reason_enum = schema["definitions"]["AuditEvent"]["properties"]["outcome"]["properties"]
        ["reason"]["enum"]
        .as_array()
        .expect("reason enum array")
        .clone();
    // The reason enum admits null (an outcome with no failure reason) alongside
    // the closed AuditFailure set.
    assert!(
        reason_enum.iter().any(serde_json::Value::is_null),
        "reason enum must admit null"
    );
    let schema_strings: Vec<String> = reason_enum
        .iter()
        .filter_map(|value| value.as_str().map(str::to_string))
        .collect();
    let expected: Vec<String> = all_failures().into_iter().map(rendered).collect();
    assert_eq!(
        schema_strings, expected,
        "schemas/datamodel.schema.json outcome.reason enum must equal the serde-rendered \
         AuditFailure variants exactly and in order"
    );
}

#[test]
fn audit_event_carries_optional_operator_with_definitions() {
    let schema = schema();
    // Optional operator (not in `required`), referencing an OperatorPrincipal.
    assert!(
        schema["definitions"]["AuditEvent"]["properties"]
            .get("operator")
            .is_some(),
        "AuditEvent must carry an optional operator property"
    );
    let required = schema["definitions"]["AuditEvent"]["required"]
        .as_array()
        .expect("required array");
    assert!(
        !required
            .iter()
            .any(|value| value.as_str() == Some("operator")),
        "operator must stay optional (None on the exchange plane)"
    );
    // The operator definitions mirror internal-api.schema.json.
    assert!(schema["definitions"].get("OperatorPrincipal").is_some());
    let mechanism_enum: Vec<String> = schema["definitions"]["OperatorAuthMechanism"]["enum"]
        .as_array()
        .expect("OperatorAuthMechanism enum")
        .iter()
        .map(|value| value.as_str().expect("string").to_string())
        .collect();
    assert_eq!(
        mechanism_enum,
        vec!["mtls", "operator_token", "shared_secret"],
        "OperatorAuthMechanism must enumerate the three operator auth mechanisms"
    );
}
