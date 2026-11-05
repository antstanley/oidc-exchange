//! Contract fixture for the `GET /internal/users/{id}/sessions` response.
//!
//! One canonical `200` body plus a structural checker, shared by the server's
//! HTTP tests and any client of the endpoint. The checker mirrors
//! `SessionList` in `schemas/internal-api.schema.json`: exactly the
//! `SessionSummary` keys, an opaque `ses_` id, and no credential-bearing field.

use serde_json::{json, Value};

/// Keys a `SessionSummary` may carry; `location` is omitted when none was recorded.
pub const SUMMARY_KEYS: [&str; 7] = [
    "id",
    "provider",
    "user_agent",
    "ip_address",
    "location",
    "created_at",
    "expires_at",
];

/// Field names that must never appear in a listing.
pub const FORBIDDEN_KEYS: [&str; 4] = [
    "refresh_token_hash",
    "device_id",
    "family_id",
    "refresh_token",
];

/// A canonical response: one located session and one without a location.
pub fn canonical_response() -> Value {
    json!({
        "sessions": [
            {
                "id": "ses_AAAAAAAAAAAAAAAAAAAAAA",
                "provider": "google",
                "user_agent": "agent/1",
                "ip_address": "203.0.113.9",
                "location": { "city": "Cape Town", "region": "Western Cape", "country": "ZA" },
                "created_at": "2026-11-05T10:00:00Z",
                "expires_at": "2026-12-05T10:00:00Z"
            },
            {
                "id": "ses_BBBBBBBBBBBBBBBBBBBBBB",
                "provider": "google",
                "user_agent": null,
                "ip_address": null,
                "created_at": "2026-11-04T10:00:00Z",
                "expires_at": "2026-12-04T10:00:00Z"
            }
        ]
    })
}

/// Panic unless `body` satisfies the listing contract.
pub fn assert_listing_contract(body: &Value) {
    let object = body.as_object().expect("body is an object");
    assert_eq!(object.len(), 1, "only the `sessions` key");
    let sessions = body["sessions"].as_array().expect("sessions is an array");
    assert!(sessions.len() <= 50, "at most 50 sessions");
    for entry in sessions {
        let entry = entry.as_object().expect("session is an object");
        for key in entry.keys() {
            assert!(SUMMARY_KEYS.contains(&key.as_str()), "unexpected key {key}");
            assert!(
                !FORBIDDEN_KEYS.contains(&key.as_str()),
                "forbidden key {key}"
            );
        }
        for required in ["id", "provider", "created_at", "expires_at"] {
            assert!(entry.contains_key(required), "missing {required}");
        }
        let id = entry["id"].as_str().expect("id is a string");
        let body = id.strip_prefix("ses_").expect("ses_ prefix");
        assert_eq!(body.len(), 22, "22 base64url characters");
        assert!(body
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
        if let Some(location) = entry.get("location") {
            let location = location.as_object().expect("location is an object");
            assert!(!location.is_empty(), "location has at least one part");
            for (key, value) in location {
                assert!(["city", "region", "country"].contains(&key.as_str()));
                let text = value.as_str().expect("part is a string");
                assert!((1..=80).contains(&text.chars().count()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_response_satisfies_the_contract() {
        assert_listing_contract(&canonical_response());
    }

    #[test]
    #[should_panic(expected = "unexpected key")]
    fn a_credential_field_violates_the_contract() {
        let mut body = canonical_response();
        body["sessions"][0]["family_id"] = json!("fam_x");
        assert_listing_contract(&body);
    }
}
