use std::collections::HashMap;

use serde_json::Value;

use oidc_exchange_core::domain::{User, UserStatus};
use oidc_exchange_core::service::claims::resolve_custom_claims;

fn make_user() -> User {
    User {
        id: "usr_123".to_string(),
        external_id: "ext_456".to_string(),
        provider: "google".to_string(),
        email: Some("alice@example.com".to_string()),
        display_name: Some("Alice".to_string()),
        metadata: HashMap::new(),
        claims: HashMap::new(),
        status: UserStatus::Active,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

#[test]
fn static_claim() {
    let user = make_user();
    let mut config_claims = HashMap::new();
    config_claims.insert("org".to_string(), "example".to_string());

    let result = resolve_custom_claims(&Some(config_claims), &user);

    assert_eq!(result.get("org"), Some(&Value::String("example".to_string())));
}

#[test]
fn field_reference_with_default_missing() {
    let user = make_user();
    let mut config_claims = HashMap::new();
    config_claims.insert(
        "role".to_string(),
        "{{ user.metadata.role | default: 'user' }}".to_string(),
    );

    let result = resolve_custom_claims(&Some(config_claims), &user);

    assert_eq!(result.get("role"), Some(&Value::String("user".to_string())));
}

#[test]
fn field_reference_with_default_present() {
    let mut user = make_user();
    user.metadata
        .insert("role".to_string(), Value::String("editor".to_string()));
    let mut config_claims = HashMap::new();
    config_claims.insert(
        "role".to_string(),
        "{{ user.metadata.role | default: 'user' }}".to_string(),
    );

    let result = resolve_custom_claims(&Some(config_claims), &user);

    assert_eq!(
        result.get("role"),
        Some(&Value::String("editor".to_string()))
    );
}

#[test]
fn per_user_claims_override_config() {
    let mut user = make_user();
    user.claims
        .insert("role".to_string(), Value::String("admin".to_string()));

    let mut config_claims = HashMap::new();
    config_claims.insert("role".to_string(), "user".to_string());

    let result = resolve_custom_claims(&Some(config_claims), &user);

    assert_eq!(
        result.get("role"),
        Some(&Value::String("admin".to_string()))
    );
}

#[test]
fn reserved_claim_rejected_from_config() {
    let user = make_user();
    let mut config_claims = HashMap::new();
    config_claims.insert("sub".to_string(), "override".to_string());
    config_claims.insert("iss".to_string(), "override".to_string());
    config_claims.insert("aud".to_string(), "override".to_string());
    config_claims.insert("iat".to_string(), "override".to_string());
    config_claims.insert("exp".to_string(), "override".to_string());
    config_claims.insert("org".to_string(), "allowed".to_string());

    let result = resolve_custom_claims(&Some(config_claims), &user);

    assert!(!result.contains_key("sub"));
    assert!(!result.contains_key("iss"));
    assert!(!result.contains_key("aud"));
    assert!(!result.contains_key("iat"));
    assert!(!result.contains_key("exp"));
    assert_eq!(result.get("org"), Some(&Value::String("allowed".to_string())));
}

#[test]
fn reserved_claim_rejected_from_user_claims() {
    let mut user = make_user();
    user.claims
        .insert("sub".to_string(), Value::String("override".to_string()));
    user.claims
        .insert("custom".to_string(), Value::String("kept".to_string()));

    let result = resolve_custom_claims(&None, &user);

    assert!(!result.contains_key("sub"));
    assert_eq!(
        result.get("custom"),
        Some(&Value::String("kept".to_string()))
    );
}

#[test]
fn missing_field_without_default_omits_claim() {
    let user = make_user();
    let mut config_claims = HashMap::new();
    config_claims.insert(
        "dept".to_string(),
        "{{ user.metadata.missing }}".to_string(),
    );

    let result = resolve_custom_claims(&Some(config_claims), &user);

    assert!(!result.contains_key("dept"));
}

#[test]
fn direct_user_field_reference() {
    let user = make_user();
    let mut config_claims = HashMap::new();
    config_claims.insert("email".to_string(), "{{ user.email }}".to_string());
    config_claims.insert("uid".to_string(), "{{ user.id }}".to_string());
    config_claims.insert("provider".to_string(), "{{ user.provider }}".to_string());

    let result = resolve_custom_claims(&Some(config_claims), &user);

    assert_eq!(
        result.get("email"),
        Some(&Value::String("alice@example.com".to_string()))
    );
    assert_eq!(
        result.get("uid"),
        Some(&Value::String("usr_123".to_string()))
    );
    assert_eq!(
        result.get("provider"),
        Some(&Value::String("google".to_string()))
    );
}

#[test]
fn no_config_claims_only_user_claims() {
    let mut user = make_user();
    user.claims
        .insert("tier".to_string(), Value::String("premium".to_string()));

    let result = resolve_custom_claims(&None, &user);

    assert_eq!(
        result.get("tier"),
        Some(&Value::String("premium".to_string()))
    );
}

#[test]
fn empty_config_and_user_claims() {
    let user = make_user();
    let result = resolve_custom_claims(&None, &user);
    assert!(result.is_empty());
}
