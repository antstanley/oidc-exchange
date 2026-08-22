use std::collections::HashMap;

use aws_sdk_dynamodb::types::AttributeValue;
use chrono::{DateTime, Utc};
use oidc_exchange_core::domain::{
    RetiredRefreshToken, Session, User, UserStatus, INITIAL_USER_VERSION,
};
use oidc_exchange_core::error::{Error, Result};

// ---------------------------------------------------------------------------
// User <-> DynamoDB Item
// ---------------------------------------------------------------------------

pub fn user_to_item(user: &User) -> HashMap<String, AttributeValue> {
    let mut item = HashMap::new();

    // Keys
    item.insert(
        "pk".to_string(),
        AttributeValue::S(format!("USER#{}", user.id)),
    );
    item.insert("sk".to_string(), AttributeValue::S("PROFILE".to_string()));

    // No GSI1 entry: lookup by provider + external_id goes through the uniqueness-guard
    // item (see `guard_to_item`) instead, so GSI1 serves only session lookups.

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
        "version".to_string(),
        AttributeValue::N(user.version.to_string()),
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
        version: get_version_or_default(item)?,
        created_at: parse_datetime(&get_s(item, "created_at")?)?,
        updated_at: parse_datetime(&get_s(item, "updated_at")?)?,
    })
}

// ---------------------------------------------------------------------------
// User uniqueness guard <-> DynamoDB Item
// ---------------------------------------------------------------------------

/// Sort key value for every user-uniqueness-guard item (see [`guard_to_item`]).
pub const GUARD_SK: &str = "UNIQUE";

/// Partition key for the uniqueness-guard item that makes `(provider, external_id)` unique.
pub fn guard_pk(provider: &str, external_id: &str) -> String {
    format!("EXT#{provider}#{external_id}")
}

/// Builds the uniqueness-guard item: `pk = EXT#<provider>#<external_id>`, `sk = UNIQUE`,
/// carrying the owning `user_id`. `create_user` writes this in the same `TransactWriteItems`
/// call as the user profile item, both conditioned on `attribute_not_exists(pk)`, so a
/// duplicate `(provider, external_id)` cancels the transaction instead of silently
/// overwriting — or racing past — an existing user.
pub fn guard_to_item(
    provider: &str,
    external_id: &str,
    user_id: &str,
) -> HashMap<String, AttributeValue> {
    let mut item = HashMap::new();
    item.insert(
        "pk".to_string(),
        AttributeValue::S(guard_pk(provider, external_id)),
    );
    item.insert("sk".to_string(), AttributeValue::S(GUARD_SK.to_string()));
    item.insert(
        "user_id".to_string(),
        AttributeValue::S(user_id.to_string()),
    );
    item
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

    // GSI1 — admin listing of a user's sessions by family. The `FAM#…` form
    // groups every generation of one sign-in under its stable family id, so
    // the listing survives rotation (the presented hash no longer identifies
    // anything after one). Revocation paths deliberately do NOT enumerate
    // through this index — it is eventually consistent and can omit a
    // session written moments earlier; they read the authoritative user-item
    // roster instead.
    item.insert(
        "GSI1pk".to_string(),
        AttributeValue::S(format!("USER#{}", session.user_id)),
    );
    item.insert(
        "GSI1sk".to_string(),
        AttributeValue::S(format!(
            "FAM#{}#SESSION#{}",
            session.family_id,
            session.created_at.to_rfc3339()
        )),
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
        "family_id".to_string(),
        AttributeValue::S(session.family_id.clone()),
    );
    item.insert(
        "generation".to_string(),
        AttributeValue::N(session.generation.to_string()),
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

    if let Some(ref rotated_at) = session.rotated_at {
        item.insert(
            "rotated_at".to_string(),
            AttributeValue::S(rotated_at.to_rfc3339()),
        );
    }

    if let Some(ref device_id) = session.device_id {
        item.insert(
            "device_id".to_string(),
            AttributeValue::S(device_id.clone()),
        );
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
    // A session item written before rotation shipped has no family attributes.
    // They read back with the same sentinel values the SQL adapters use for a
    // NULL `family_id` column — an empty string that deliberately fails
    // `is_valid_family_id`, so downstream family operations visibly fail
    // rather than silently matching a family that does not exist. The
    // `generation` default mirrors `get_version_or_default`'s migration
    // handling of the pre-`version` user items.
    let family_id = get_s_opt(item, "family_id").unwrap_or_default();
    let rotated_at = match get_s_opt(item, "rotated_at") {
        Some(s) => Some(parse_datetime(&s)?),
        None => None,
    };

    Ok(Session {
        user_id: get_s(item, "user_id")?,
        refresh_token_hash: get_s(item, "refresh_token_hash")?,
        family_id,
        generation: get_generation_or_default(item)?,
        provider: get_s(item, "provider")?,
        expires_at: parse_datetime(&get_s(item, "expires_at")?)?,
        rotated_at,
        device_id: get_s_opt(item, "device_id"),
        user_agent: get_s_opt(item, "user_agent"),
        ip_address: get_s_opt(item, "ip_address"),
        created_at: parse_datetime(&get_s(item, "created_at")?)?,
    })
}

// ---------------------------------------------------------------------------
// Retired refresh token <-> DynamoDB Item
// ---------------------------------------------------------------------------

/// Sort key value for every retired-refresh-token item.
pub const RETIRED_SK: &str = "RETIRED";

/// Builds the retirement-record item: `pk = RETIRED#<hash>`, `sk = RETIRED`,
/// GSI1 filed under the owning user with a family-grouped sort key, and the
/// same numeric `ttl` attribute sessions carry so DynamoDB reaps records
/// natively when their retention deadline passes.
pub fn retired_to_item(record: &RetiredRefreshToken) -> HashMap<String, AttributeValue> {
    let mut item = HashMap::new();

    item.insert(
        "pk".to_string(),
        AttributeValue::S(format!("RETIRED#{}", record.refresh_token_hash)),
    );
    item.insert("sk".to_string(), AttributeValue::S(RETIRED_SK.to_string()));

    item.insert(
        "GSI1pk".to_string(),
        AttributeValue::S(format!("USER#{}", record.user_id)),
    );
    item.insert(
        "GSI1sk".to_string(),
        AttributeValue::S(format!(
            "FAM#{}#RETIRED#{}",
            record.family_id,
            record.retired_at.to_rfc3339()
        )),
    );

    item.insert(
        "refresh_token_hash".to_string(),
        AttributeValue::S(record.refresh_token_hash.clone()),
    );
    item.insert(
        "family_id".to_string(),
        AttributeValue::S(record.family_id.clone()),
    );
    item.insert(
        "user_id".to_string(),
        AttributeValue::S(record.user_id.clone()),
    );
    item.insert(
        "successor_hash".to_string(),
        AttributeValue::S(record.successor_hash.clone()),
    );
    item.insert(
        "retired_at".to_string(),
        AttributeValue::S(record.retired_at.to_rfc3339()),
    );
    item.insert(
        "expires_at".to_string(),
        AttributeValue::S(record.expires_at.to_rfc3339()),
    );
    // TTL for DynamoDB automatic expiration (epoch seconds).
    item.insert(
        "ttl".to_string(),
        AttributeValue::N(record.expires_at.timestamp().to_string()),
    );

    item
}

/// Parses a retirement-record item. Every attribute is written by
/// [`retired_to_item`], so a missing or mistyped one is corruption at the
/// store-read boundary and surfaces as a store error rather than a
/// half-record that could silently disarm reuse detection.
pub fn item_to_retired(item: &HashMap<String, AttributeValue>) -> Result<RetiredRefreshToken> {
    Ok(RetiredRefreshToken {
        refresh_token_hash: get_s(item, "refresh_token_hash")?,
        family_id: get_s(item, "family_id")?,
        user_id: get_s(item, "user_id")?,
        successor_hash: get_s(item, "successor_hash")?,
        retired_at: parse_datetime(&get_s(item, "retired_at")?)?,
        expires_at: parse_datetime(&get_s(item, "expires_at")?)?,
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
    item.get(key)
        .and_then(|v| v.as_s().ok())
        .map(|s| s.to_string())
}

/// Reads the `version` attribute, treating a missing attribute (an item written before
/// the field existed) as [`INITIAL_USER_VERSION`] — the migration default.
fn get_version_or_default(item: &HashMap<String, AttributeValue>) -> Result<u64> {
    match item.get("version") {
        Some(v) => v
            .as_n()
            .map_err(|_| Error::StoreError {
                detail: "invalid attribute: version".to_string(),
            })?
            .parse::<u64>()
            .map_err(|e| Error::StoreError {
                detail: format!("invalid version: {e}"),
            }),
        None => Ok(INITIAL_USER_VERSION),
    }
}

/// Reads the session item's `generation` attribute, treating a missing
/// attribute (an item written before rotation shipped) as generation 0 — the
/// migration default for pre-rotation rows. A present-but-non-numeric value is
/// a distinct corruption failure and is rejected, not defaulted.
fn get_generation_or_default(item: &HashMap<String, AttributeValue>) -> Result<u32> {
    match item.get("generation") {
        Some(v) => v
            .as_n()
            .map_err(|_| Error::StoreError {
                detail: "invalid attribute: generation".to_string(),
            })?
            .parse::<u32>()
            .map_err(|e| Error::StoreError {
                detail: format!("invalid generation: {e}"),
            }),
        None => Ok(0),
    }
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

// ---------------------------------------------------------------------------
// The authoritative per-user session roster
// ---------------------------------------------------------------------------

/// One family's entry in the user item's authoritative `families` map:
/// which generation is currently live and every generation hash (live plus
/// retired, until revocation) the family has ever held. This is what makes
/// `revoke_family` complete without trusting an eventually consistent index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FamilyRoster {
    /// The live generation's hash, or an empty string when the family
    /// currently has none (fully revoked or not yet rotated into existence).
    pub live: String,
    /// Every generation hash of this family still remembered: the live one
    /// plus each retained retirement record.
    pub members: Vec<String>,
}

impl FamilyRoster {
    pub fn new(live: String, members: Vec<String>) -> Self {
        Self { live, members }
    }

    /// Serialize to the nested map attribute stored under `families.<id>`.
    pub fn to_attribute(&self) -> AttributeValue {
        AttributeValue::M(HashMap::from([
            ("live".to_string(), AttributeValue::S(self.live.clone())),
            (
                "members".to_string(),
                AttributeValue::Ss(self.members.clone()),
            ),
        ]))
    }

    /// Parse one `families.<id>` entry; a malformed entry is roster
    /// corruption and must surface rather than silently truncate the
    /// revocation set.
    pub fn from_attribute(value: &AttributeValue) -> Result<Self> {
        let map = value.as_m().map_err(|_| Error::StoreError {
            detail: "roster families entry is not a map".to_string(),
        })?;
        let live = map
            .get("live")
            .and_then(|v| v.as_s().ok())
            .ok_or_else(|| Error::StoreError {
                detail: "roster families entry is missing its live pointer".to_string(),
            })?
            .clone();
        let members = map
            .get("members")
            .and_then(|v| v.as_ss().ok())
            .ok_or_else(|| Error::StoreError {
                detail: "roster families entry is missing its member set".to_string(),
            })?
            .clone();
        Ok(Self { live, members })
    }
}

/// The session-revocation roster carried by the user item (`pk = USER#<id>`,
/// `sk = PROFILE`): the `sessions` string set naming every live generation,
/// and the `families` map grouping each family's generations. Every session
/// write maintains both inside the same `TransactWriteItems` as the session
/// items themselves, so a strongly consistent read of this struct is a
/// complete picture of a user's credential state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserRoster {
    pub sessions: Vec<String>,
    /// Keyed by family id.
    pub families: HashMap<String, FamilyRoster>,
}

impl UserRoster {
    /// Extract the roster from a user item. Both attributes are absent on
    /// users with no sessions yet (and on pre-rotation user items), which
    /// reads as an empty roster, not an error.
    pub fn from_item(item: &HashMap<String, AttributeValue>) -> Result<Self> {
        let sessions = match item.get("sessions") {
            Some(v) => v.as_ss().map_err(|_| Error::StoreError {
                detail: "user item attribute sessions is not a string set".to_string(),
            })?,
            None => &[],
        }
        .to_vec();

        let mut families = HashMap::new();
        if let Some(value) = item.get("families") {
            let map = value.as_m().map_err(|_| Error::StoreError {
                detail: "user item attribute families is not a map".to_string(),
            })?;
            for (family_id, entry) in map {
                families.insert(family_id.clone(), FamilyRoster::from_attribute(entry)?);
            }
        }

        debug_assert!(
            families.values().all(|f| f.members.iter().all(|m| !m.is_empty())),
            "roster family members may not be empty strings"
        );

        Ok(Self { sessions, families })
    }

    /// Look up which family owns `hash` as its live generation.
    pub fn live_family_of(&self, hash: &str) -> Option<&FamilyRoster> {
        self.families
            .values()
            .find(|family| family.live == hash)
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
            version: 3,
            created_at: now,
            updated_at: now,
        }
    }

    fn sample_session() -> Session {
        let now = Utc::now();
        Session {
            user_id: "usr_01abc".to_string(),
            refresh_token_hash: "sha256_deadbeef".to_string(),
            family_id: "fam_0000000000000000000000000a".to_string(),
            generation: 0,
            provider: "google".to_string(),
            expires_at: now + chrono::Duration::hours(24),
            rotated_at: None,
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
        assert_eq!(user.version, restored.version);
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
            version: INITIAL_USER_VERSION,
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
        assert_eq!(INITIAL_USER_VERSION, restored.version);
    }

    #[test]
    fn user_item_has_version_attribute() {
        let user = sample_user();
        let item = user_to_item(&user);

        let version = item.get("version").expect("item should have version");
        let version_val: u64 = version
            .as_n()
            .expect("version should be N")
            .parse()
            .expect("version should be a valid u64");
        assert_eq!(version_val, user.version);
    }

    #[test]
    fn item_to_user_missing_version_defaults_to_initial_version() {
        let user = sample_user();
        let mut item = user_to_item(&user);
        item.remove("version");

        let restored = item_to_user(&item).expect("should parse user from item");
        assert_eq!(restored.version, INITIAL_USER_VERSION);
        // Sanity: every other field still round-trips even without the version attribute.
        assert_eq!(restored.id, user.id);
    }

    /// Negative-space: a `version` attribute present but not a DynamoDB `N` (number) is a
    /// distinct failure from "missing" and must be rejected, not silently defaulted.
    #[test]
    fn item_to_user_non_numeric_version_returns_error() {
        let user = sample_user();
        let mut item = user_to_item(&user);
        item.insert(
            "version".to_string(),
            AttributeValue::S("not-a-number".to_string()),
        );

        let result = item_to_user(&item);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("version"));
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

    /// The session round trip must preserve the family identity rotation is
    /// built on: `family_id`, `generation`, and `rotated_at` survive the item
    /// mapping unchanged.
    #[test]
    fn session_round_trip_preserves_family_fields() {
        let mut session = sample_session();
        session.generation = 7;
        session.rotated_at = Some(Utc::now());

        let item = session_to_item(&session);
        let restored = item_to_session(&item).expect("should parse session from item");

        assert_eq!(session.family_id, restored.family_id);
        assert_eq!(session.generation, restored.generation);
        assert_eq!(
            session.rotated_at.map(|ts| ts.timestamp_millis()),
            restored.rotated_at.map(|ts| ts.timestamp_millis())
        );
    }

    /// A session item written before rotation shipped (no family attributes)
    /// must read back with the migration defaults — empty-string family,
    /// generation 0, no rotation timestamp — mirroring
    /// `item_to_user_missing_version_defaults_to_initial_version`.
    #[test]
    fn item_to_session_missing_family_attributes_defaults_to_legacy_shape() {
        // Build from a fully-populated session (rotated_at set) so every
        // family attribute is present in the item, then strip all three to
        // simulate the pre-rotation record.
        let mut rotated = sample_session();
        rotated.generation = 3;
        rotated.rotated_at = Some(Utc::now());
        let mut item = session_to_item(&rotated);
        assert!(item.remove("family_id").is_some());
        assert!(item.remove("generation").is_some());
        assert!(item.remove("rotated_at").is_some());

        let restored = item_to_session(&item).expect("legacy session item must parse");
        assert_eq!(
            restored.family_id, "",
            "missing family must land on the empty-string sentinel"
        );
        assert!(!oidc_exchange_core::domain::is_valid_family_id(
            &restored.family_id
        ));
        assert_eq!(restored.generation, 0);
        assert_eq!(restored.rotated_at, None);
        // Sanity: every other field still round-trips without the family attrs.
        assert_eq!(restored.refresh_token_hash, "sha256_deadbeef");
    }

    /// Negative-space: a `generation` attribute present but not a DynamoDB `N`
    /// is a distinct corruption failure and must be rejected, not silently
    /// defaulted to 0.
    #[test]
    fn item_to_session_non_numeric_generation_returns_error() {
        let mut item = session_to_item(&sample_session());
        item.insert(
            "generation".to_string(),
            AttributeValue::S("not-a-number".to_string()),
        );

        let result = item_to_session(&item);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("generation"));
    }

    #[test]
    fn session_round_trip_no_optional_fields() {
        let now = Utc::now();
        let session = Session {
            user_id: "usr_01abc".to_string(),
            refresh_token_hash: "sha256_cafe".to_string(),
            family_id: "fam_0000000000000000000000000b".to_string(),
            generation: 0,
            provider: "atproto".to_string(),
            expires_at: now + chrono::Duration::hours(1),
            rotated_at: None,
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
        assert!(
            !item.contains_key("GSI1pk"),
            "User item must not carry a GSI1pk — lookup goes through the uniqueness guard"
        );
        assert!(
            !item.contains_key("GSI1sk"),
            "User item must not carry a GSI1sk — lookup goes through the uniqueness guard"
        );
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
