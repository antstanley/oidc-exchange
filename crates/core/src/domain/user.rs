use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The `version` value every `create_user` writes, and the value a read of a
/// pre-migration row/item (one with no `version` attribute/column) defaults to.
pub const INITIAL_USER_VERSION: u64 = 1;

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
    /// Store-managed optimistic-concurrency counter; never caller-supplied.
    /// `create_user` writes [`INITIAL_USER_VERSION`]; every `update_user` increments it.
    pub version: u64,
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

impl UserStatus {
    /// Whether a status patch from `self` to `target` is a valid lifecycle transition.
    ///
    /// `Deleted` is strictly terminal: it is reachable from any other status, but it has no
    /// outgoing edge at all, not even back to itself. A patch that repeats the current status
    /// is accepted as a no-op everywhere except on `Deleted`, where every patch — including
    /// `Deleted -> Deleted` — is rejected. `Active` and `Suspended` otherwise transition freely
    /// between each other. Every pair not covered by these rules is rejected.
    pub fn can_transition_to(&self, target: &UserStatus) -> bool {
        match (self, target) {
            // Deleted has no outgoing edge, not even to itself.
            (UserStatus::Deleted, _) => false,
            // Deleted is reachable from any other (non-Deleted, by the arm above) status.
            (_, UserStatus::Deleted) => true,
            // Active <-> Suspended, and same-status no-ops, are the remaining allowed edges.
            (
                UserStatus::Active | UserStatus::Suspended,
                UserStatus::Active | UserStatus::Suspended,
            ) => true,
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Full ordered-pair truth table for `can_transition_to`, covering all nine
    /// `(current, target)` combinations of the three `UserStatus` variants. Every edge drawn
    /// on the lifecycle diagram (self-loops on `Active`/`Suspended`, `Active <-> Suspended`,
    /// and `* -> Deleted` for non-`Deleted` `*`) maps to `true`; every off-diagram pair,
    /// including every outgoing edge from `Deleted`, maps to `false`.
    #[test]
    fn can_transition_to_matches_full_truth_table() {
        use UserStatus::{Active, Deleted, Suspended};

        let cases: [(UserStatus, UserStatus, bool); 9] = [
            (Active, Active, true),
            (Active, Suspended, true),
            (Active, Deleted, true),
            (Suspended, Active, true),
            (Suspended, Suspended, true),
            (Suspended, Deleted, true),
            (Deleted, Active, false),
            (Deleted, Suspended, false),
            (Deleted, Deleted, false),
        ];

        for (current, target, expected) in &cases {
            assert_eq!(
                current.can_transition_to(target),
                *expected,
                "expected {current:?} -> {target:?} to be {expected}"
            );
        }
    }

    /// Negative-space coverage called out explicitly in the definition of done: `Deleted` has
    /// no outgoing edge at all (not even back to itself), while `Suspended -> Deleted` — the
    /// edge that makes a suspended user deletable — is allowed.
    #[test]
    fn deleted_is_strictly_terminal() {
        assert!(!UserStatus::Deleted.can_transition_to(&UserStatus::Active));
        assert!(!UserStatus::Deleted.can_transition_to(&UserStatus::Suspended));
        assert!(!UserStatus::Deleted.can_transition_to(&UserStatus::Deleted));
        assert!(UserStatus::Suspended.can_transition_to(&UserStatus::Deleted));
    }
}
