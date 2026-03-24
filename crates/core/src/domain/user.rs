use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    /// Internal ID, e.g., "usr_01ARZ3NDEK..."
    pub id: String,
    /// Provider's sub claim / DID
    pub external_id: String,
    /// "google", "apple", "atproto"
    pub provider: String,
    /// Not all providers guarantee email
    pub email: Option<String>,
    pub display_name: Option<String>,
    /// Extensible fields from sync
    pub metadata: HashMap<String, Value>,
    /// Per-user private claims added to access token JWT
    pub claims: HashMap<String, Value>,
    pub status: UserStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UserStatus {
    Active,
    /// Can't get new tokens, existing tokens still valid until expiry
    Suspended,
    /// Soft delete, all sessions revoked
    Deleted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewUser {
    pub external_id: String,
    pub provider: String,
    pub email: Option<String>,
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserPatch {
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub metadata: Option<HashMap<String, Value>>,
    /// Replace entire claims map
    pub claims: Option<HashMap<String, Value>>,
    pub status: Option<UserStatus>,
}
