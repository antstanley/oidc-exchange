use std::collections::HashMap;

use aws_sdk_dynamodb::types::AttributeValue;
use chrono::{DateTime, Utc};
use oidc_exchange_core::domain::{Session, User, UserStatus};
use oidc_exchange_core::error::{Error, Result};

// ---------------------------------------------------------------------------
// User <-> DynamoDB Item
// ---------------------------------------------------------------------------

pub fn user_to_item(user: &User) -> HashMap<String, AttributeValue> {
    let mut item = HashMap::new();

    // Keys
    item.insert("pk".to_string(), AttributeValue::S(format!("USER#{}", user.id)));
    item.insert("sk".to_string(), AttributeValue::S("PROFILE".to_string()));

    // GSI1 — lookup by external_id
    item.insert(
        "GSI1pk".to_string(),
        AttributeValue::S(format!("EXT#{}", user.external_id)),
    );
    item.insert("GSI1sk".to_string(), AttributeValue::S("USER".to_string()));

    // Data attributes
    item.insert("id".to_string(), AttributeValue::S(user.id.clone()));
    item.insert(
        "external_id".to_string(),
        AttributeValue::S(user.external_id.clone()),
    );
    item.insert(
        "provider".to_string(),
        AttributeValue::S(user.provider.clone()),
    );

    if let Some(ref email) = user.email {
        item.insert("email".to_string(), AttributeValue::S(email.clone()));
    }
    if let Some(ref display_name) = user.display_name {
        item.insert(
            "display_name".to_string(),
            AttributeValue::S(display_name.clone()),
        );
    }

    // Serialize metadata and claims as JSON strings
    item.insert(
        "metadata".to_string(),
        AttributeValue::S(serde_json::to_string(&user.metadata).unwrap_or_default()),
    );
    item.insert(
        "claims".to_string(),
        AttributeValue::S(serde_json::to_string(&user.claims).unwrap_or_default()),
    );

    // Status as lowercase string
    item.insert(
        "status".to_string(),
        AttributeValue::S(status_to_string(&user.status)),
    );

    item.insert(
        "created_at".to_string(),
        AttributeValue::S(user.created_at.to_rfc3339()),
    );
    item.insert(
        "updated_at".to_string(),
        AttributeValue::S(user.updated_at.to_rfc3339()),
    );

    item
}

pub fn item_to_user(item: &HashMap<String, AttributeValue>) -> Result<User> {
    Ok(User {
        id: get_s(item, "id")?,
        external_id: get_s(item, "external_id")?,
        provider: get_s(item, "provider")?,
        email: get_s_opt(item, "email"),
        display_name: get_s_opt(item, "display_name"),
        metadata: get_json_map(item, "metadata")?,
        claims: get_json_map(item, "claims")?,
        status: string_to_status(&get_s(item, "status")?)?,
        created_at: parse_datetime(&get_s(item, "created_at")?)?,
        updated_at: parse_datetime(&get_s(item, "updated_at")?)?,
    })
}

// ---------------------------------------------------------------------------
// Session <-> DynamoDB Item
// ---------------------------------------------------------------------------

pub fn session_to_item(session: &Session) -> HashMap<String, AttributeValue> {
    let mut item = HashMap::new();

    // Keys
    item.insert(
        "pk".to_string(),
        AttributeValue::S(format!("SESSION#{}", session.refresh_token_hash)),
    );
    item.insert("sk".to_string(), AttributeValue::S("SESSION".to_string()));

    // GSI1 — list sessions by user
    item.insert(
        "GSI1pk".to_string(),
        AttributeValue::S(format!("USER#{}", session.user_id)),
    );
    item.insert(
        "GSI1sk".to_string(),
        AttributeValue::S(format!("SESSION#{}", session.created_at.to_rfc3339())),
    );

    // Data attributes
    item.insert(
        "user_id".to_string(),
        AttributeValue::S(session.user_id.clone()),
    );
    item.insert(
        "refresh_token_hash".to_string(),
        AttributeValue::S(session.refresh_token_hash.clone()),
    );
    item.insert(
        "provider".to_string(),
        AttributeValue::S(session.provider.clone()),
    );
    item.insert(
        "expires_at".to_string(),
        AttributeValue::S(session.expires_at.to_rfc3339()),
    );
    item.insert(
        "created_at".to_string(),
        AttributeValue::S(session.created_at.to_rfc3339()),
    );

    if let Some(ref device_id) = session.device_id {
        item.insert("device_id".to_string(), AttributeValue::S(device_id.clone()));
    }
    if let Some(ref user_agent) = session.user_agent {
        item.insert(
            "user_agent".to_string(),
            AttributeValue::S(user_agent.clone()),
        );
    }
    if let Some(ref ip_address) = session.ip_address {
        item.insert(
            "ip_address".to_string(),
            AttributeValue::S(ip_address.clone()),
        );
    }

    // TTL for DynamoDB automatic expiration (epoch seconds)
    item.insert(
        "ttl".to_string(),
        AttributeValue::N(session.expires_at.timestamp().to_string()),
    );

    item
}

pub fn item_to_session(item: &HashMap<String, AttributeValue>) -> Result<Session> {
    Ok(Session {
        user_id: get_s(item, "user_id")?,
        refresh_token_hash: get_s(item, "refresh_token_hash")?,
        provider: get_s(item, "provider")?,
        expires_at: parse_datetime(&get_s(item, "expires_at")?)?,
        device_id: get_s_opt(item, "device_id"),
        user_agent: get_s_opt(item, "user_agent"),
        ip_address: get_s_opt(item, "ip_address"),
        created_at: parse_datetime(&get_s(item, "created_at")?)?,
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn get_s(item: &HashMap<String, AttributeValue>, key: &str) -> Result<String> {
    item.get(key)
        .and_then(|v| v.as_s().ok())
        .map(|s| s.to_string())
        .ok_or_else(|| Error::StoreError {
            detail: format!("missing or invalid attribute: {key}"),
        })
}

fn get_s_opt(item: &HashMap<String, AttributeValue>, key: &str) -> Option<String> {
    item.get(key).and_then(|v| v.as_s().ok()).map(|s| s.to_string())
}

fn get_json_map(
    item: &HashMap<String, AttributeValue>,
    key: &str,
) -> Result<HashMap<String, serde_json::Value>> {
    match item.get(key).and_then(|v| v.as_s().ok()) {
        Some(s) => serde_json::from_str(s).map_err(|e| Error::StoreError {
            detail: format!("invalid JSON in {key}: {e}"),
        }),
        None => Ok(HashMap::new()),
    }
}

fn parse_datetime(s: &str) -> Result<DateTime<Utc>> {
    s.parse::<DateTime<Utc>>().map_err(|e| Error::StoreError {
        detail: format!("invalid datetime: {e}"),
    })
}

fn status_to_string(status: &UserStatus) -> String {
    match status {
        UserStatus::Active => "active".to_string(),
        UserStatus::Suspended => "suspended".to_string(),
        UserStatus::Deleted => "deleted".to_string(),
    }
}

fn string_to_status(s: &str) -> Result<UserStatus> {
    match s {
        "active" => Ok(UserStatus::Active),
        "suspended" => Ok(UserStatus::Suspended),
        "deleted" => Ok(UserStatus::Deleted),
        _ => Err(Error::StoreError {
            detail: format!("unknown user status: {s}"),
        }),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::collections::HashMap;

    fn sample_user() -> User {
        let now = Utc::now();
        let mut metadata = HashMap::new();
        metadata.insert(
            "role".to_string(),
            serde_json::Value::String("admin".to_string()),
        );
        let mut claims = HashMap::new();
        claims.insert(
            "org_id".to_string(),
            serde_json::Value::String("org_123".to_string()),
        );

        User {
            id: "usr_01abc".to_string(),
            external_id: "google|12345".to_string(),
            provider: "google".to_string(),
            email: Some("alice@example.com".to_string()),
            display_name: Some("Alice".to_string()),
            metadata,
            claims,
            status: UserStatus::Active,
            created_at: now,
            updated_at: now,
        }
    }

    fn sample_session() -> Session {
        let now = Utc::now();
        Session {
            user_id: "usr_01abc".to_string(),
            refresh_token_hash: "sha256_deadbeef".to_string(),
            provider: "google".to_string(),
            expires_at: now + chrono::Duration::hours(24),
            device_id: Some("device_1".to_string()),
            user_agent: Some("Mozilla/5.0".to_string()),
            ip_address: Some("127.0.0.1".to_string()),
            created_at: now,
        }
    }

    #[test]
    fn user_round_trip() {
        let user = sample_user();
        let item = user_to_item(&user);
        let restored = item_to_user(&item).expect("should parse user from item");

        assert_eq!(user.id, restored.id);
        assert_eq!(user.external_id, restored.external_id);
        assert_eq!(user.provider, restored.provider);
        assert_eq!(user.email, restored.email);
        assert_eq!(user.display_name, restored.display_name);
        assert_eq!(user.metadata, restored.metadata);
        assert_eq!(user.claims, restored.claims);
        assert_eq!(user.status, restored.status);
        // Datetime round-trip may lose sub-nanosecond precision, compare timestamps
        assert_eq!(
            user.created_at.timestamp_millis(),
            restored.created_at.timestamp_millis()
        );
        assert_eq!(
            user.updated_at.timestamp_millis(),
            restored.updated_at.timestamp_millis()
        );
    }

    #[test]
    fn user_round_trip_no_optional_fields() {
        let now = Utc::now();
        let user = User {
            id: "usr_02xyz".to_string(),
            external_id: "apple|99999".to_string(),
            provider: "apple".to_string(),
            email: None,
            display_name: None,
            metadata: HashMap::new(),
            claims: HashMap::new(),
            status: UserStatus::Suspended,
            created_at: now,
            updated_at: now,
        };

        let item = user_to_item(&user);
        let restored = item_to_user(&item).expect("should parse user from item");

        assert_eq!(user.id, restored.id);
        assert_eq!(user.email, restored.email);
        assert_eq!(user.display_name, restored.display_name);
        assert!(restored.metadata.is_empty());
        assert!(restored.claims.is_empty());
        assert_eq!(UserStatus::Suspended, restored.status);
    }

    #[test]
    fn session_round_trip() {
        let session = sample_session();
        let item = session_to_item(&session);
        let restored = item_to_session(&item).expect("should parse session from item");

        assert_eq!(session.user_id, restored.user_id);
        assert_eq!(session.refresh_token_hash, restored.refresh_token_hash);
        assert_eq!(session.provider, restored.provider);
        assert_eq!(session.device_id, restored.device_id);
        assert_eq!(session.user_agent, restored.user_agent);
        assert_eq!(session.ip_address, restored.ip_address);
        assert_eq!(
            session.expires_at.timestamp_millis(),
            restored.expires_at.timestamp_millis()
        );
        assert_eq!(
            session.created_at.timestamp_millis(),
            restored.created_at.timestamp_millis()
        );
    }

    #[test]
    fn session_round_trip_no_optional_fields() {
        let now = Utc::now();
        let session = Session {
            user_id: "usr_01abc".to_string(),
            refresh_token_hash: "sha256_cafe".to_string(),
            provider: "atproto".to_string(),
            expires_at: now + chrono::Duration::hours(1),
            device_id: None,
            user_agent: None,
            ip_address: None,
            created_at: now,
        };

        let item = session_to_item(&session);
        let restored = item_to_session(&item).expect("should parse session from item");

        assert_eq!(session.user_id, restored.user_id);
        assert_eq!(session.device_id, restored.device_id);
        assert_eq!(session.user_agent, restored.user_agent);
        assert_eq!(session.ip_address, restored.ip_address);
    }

    #[test]
    fn session_item_has_ttl() {
        let session = sample_session();
        let item = session_to_item(&session);

        let ttl = item.get("ttl").expect("item should have ttl");
        let ttl_val: i64 = ttl
            .as_n()
            .expect("ttl should be N")
            .parse()
            .expect("ttl should be valid i64");
        assert_eq!(ttl_val, session.expires_at.timestamp());
    }

    #[test]
    fn user_item_has_correct_keys() {
        let user = sample_user();
        let item = user_to_item(&user);

        assert_eq!(
            item.get("pk").unwrap().as_s().unwrap(),
            &format!("USER#{}", user.id)
        );
        assert_eq!(item.get("sk").unwrap().as_s().unwrap(), "PROFILE");
        assert_eq!(
            item.get("GSI1pk").unwrap().as_s().unwrap(),
            &format!("EXT#{}", user.external_id)
        );
        assert_eq!(item.get("GSI1sk").unwrap().as_s().unwrap(), "USER");
    }

    #[test]
    fn session_item_has_correct_keys() {
        let session = sample_session();
        let item = session_to_item(&session);

        assert_eq!(
            item.get("pk").unwrap().as_s().unwrap(),
            &format!("SESSION#{}", session.refresh_token_hash)
        );
        assert_eq!(item.get("sk").unwrap().as_s().unwrap(), "SESSION");
        assert_eq!(
            item.get("GSI1pk").unwrap().as_s().unwrap(),
            &format!("USER#{}", session.user_id)
        );
        assert!(item
            .get("GSI1sk")
            .unwrap()
            .as_s()
            .unwrap()
            .starts_with("SESSION#"));
    }

    #[test]
    fn item_to_user_missing_field_returns_error() {
        let item = HashMap::new();
        let result = item_to_user(&item);
        assert!(result.is_err());
    }

    #[test]
    fn item_to_session_missing_field_returns_error() {
        let item = HashMap::new();
        let result = item_to_session(&item);
        assert!(result.is_err());
    }
}
