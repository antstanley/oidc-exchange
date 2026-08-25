use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Error, Result};

/// The `version` value every `create_user` writes, and the value a read of a
/// pre-migration row/item (one with no `version` attribute/column) defaults to.
pub const INITIAL_USER_VERSION: u64 = 1;

/// The largest page size any admin listing may return.
///
/// Every admin read is bounded by this constant: a caller asking for more rows
/// than [`MAX_ADMIN_PAGE_SIZE`] gets a 200-row page rather than an unbounded
/// one, so no caller configuration can turn `/internal/users` into a
/// full-table materialization. The clamp runs in the core (see
/// [`clamp_admin_page_limit`]) — never in a handler — so *every* path to an
/// adapter is bounded, including paths that bypass HTTP.
pub const MAX_ADMIN_PAGE_SIZE: u32 = 200;

/// Page size used when an admin-listing caller omits `limit` (the value the
/// published internal-API schema documents as the default).
pub const DEFAULT_ADMIN_PAGE_SIZE: u32 = 50;

/// Clamp a caller-supplied admin page size into the contract bounds.
///
/// `0` is rejected — the published schema documents `minimum: 1`, and a
/// zero-row request is a caller bug worth surfacing, not a page shape to
/// invent. Above-bound requests are clamped down to
/// [`MAX_ADMIN_PAGE_SIZE`] — the documented server-side clamp that keeps the
/// response bounded. Returns the effective limit for the read.
pub fn clamp_admin_page_limit(limit: u32) -> Result<u32> {
    if limit == 0 {
        return Err(Error::InvalidRequest {
            reason: format!("limit must be at least 1 and at most {MAX_ADMIN_PAGE_SIZE}"),
        });
    }
    Ok(limit.min(MAX_ADMIN_PAGE_SIZE))
}

/// One bounded page of a cursor-paginated user listing.
///
/// `next_cursor` is opaque and adapter-issued: `None` means the listing is
/// exhausted and is the *only* completion signal — a page shorter than the
/// requested limit may still carry a non-null cursor (on DynamoDB the scan
/// `Limit` applies before the status filter), so callers page until
/// `next_cursor` is null, never until a short page. Serialized with an
/// explicit JSON `null` (not omitted) so the wire contract's
/// `"next_cursor": null` exhaustion marker survives round-trips.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserPage {
    pub users: Vec<User>,
    pub next_cursor: Option<String>,
}

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

    /// The page-limit clamp at all three validity boundaries (one below, at, one above the
    /// documented maximum): below the minimum rejects, at the maximum passes through
    /// unchanged, above the maximum clamps down to [`MAX_ADMIN_PAGE_SIZE`].
    #[test]
    fn clamp_admin_page_limit_enforces_documented_bounds() {
        assert!(
            clamp_admin_page_limit(0).is_err(),
            "limit 0 must be rejected"
        );
        assert_eq!(
            clamp_admin_page_limit(1).expect("1 is the minimum valid limit"),
            1
        );
        assert_eq!(
            clamp_admin_page_limit(MAX_ADMIN_PAGE_SIZE).expect("the maximum itself is valid"),
            MAX_ADMIN_PAGE_SIZE
        );
        assert_eq!(
            clamp_admin_page_limit(MAX_ADMIN_PAGE_SIZE + 1)
                .expect("above-bound requests clamp rather than error"),
            MAX_ADMIN_PAGE_SIZE,
            "an above-bound request must be clamped to MAX_ADMIN_PAGE_SIZE"
        );
        assert_eq!(
            clamp_admin_page_limit(u32::MAX).expect("u32::MAX still fits the clamp"),
            MAX_ADMIN_PAGE_SIZE
        );
    }

    /// The rejected zero case carries a reason naming the bounds, so an operator reading
    /// the API error can correct the request without consulting the schema.
    #[test]
    fn clamp_admin_page_limit_zero_error_names_the_bounds() {
        let err = clamp_admin_page_limit(0).expect_err("zero must be rejected");
        match &err {
            Error::InvalidRequest { reason } => {
                assert!(
                    reason.contains(&MAX_ADMIN_PAGE_SIZE.to_string()),
                    "the reason must name the maximum, got: {reason}"
                );
                assert!(
                    reason.contains("at least 1"),
                    "the reason must name the minimum, got: {reason}"
                );
            }
            other => panic!("expected Error::InvalidRequest, got {other:?}"),
        }
    }

    /// `next_cursor` serializes as an explicit JSON `null` when exhausted — never as an
    /// omitted field — because the published contract makes `"next_cursor": null` the only
    /// completion signal and generated clients branch on the field's presence-in-JSON value.
    #[test]
    fn user_page_next_cursor_serializes_as_explicit_null() {
        let page = UserPage {
            users: Vec::new(),
            next_cursor: None,
        };
        let value: serde_json::Value = serde_json::to_value(&page).expect("serialize UserPage");
        assert!(
            value.get("next_cursor").is_some(),
            "next_cursor must be present in JSON even when null"
        );
        assert!(value["next_cursor"].is_null());

        let with_cursor = UserPage {
            users: Vec::new(),
            next_cursor: Some("opaque-cursor".to_string()),
        };
        let value: serde_json::Value =
            serde_json::to_value(&with_cursor).expect("serialize UserPage");
        assert_eq!(value["next_cursor"], "opaque-cursor");
        assert!(value["users"].is_array());
    }
}
