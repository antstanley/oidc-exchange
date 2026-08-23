use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::secret::Secret;

#[derive(Clone, Serialize, Deserialize)]
pub struct Session {
    pub user_id: String,
    /// SHA-256 hash of the opaque token
    pub refresh_token_hash: Secret<String>,
    pub provider: String,
    pub expires_at: DateTime<Utc>,
    pub device_id: Option<String>,
    pub user_agent: Option<String>,
    pub ip_address: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl std::fmt::Debug for Session {
    /// Hand-written so the session lookup key renders as `<redacted>` no matter who
    /// formats the struct: `refresh_token_hash` is a `Secret`, which has no `Debug` impl
    /// of its own to lean on, and `derive(Debug)` here would not compile anyway. The
    /// remaining fields pass through — they are identifiers and client-asserted
    /// provenance, permitted in diagnostics (though never as span *values*; see the
    /// adapters' `skip(...)` instrumentation).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("user_id", &self.user_id)
            .field("refresh_token_hash", &"<redacted>")
            .field("provider", &self.provider)
            .field("expires_at", &self.expires_at)
            .field("device_id", &self.device_id)
            .field("user_agent", &self.user_agent)
            .field("ip_address", &self.ip_address)
            .field("created_at", &self.created_at)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_session() -> Session {
        let now = Utc::now();
        Session {
            user_id: "usr_debug_test".to_string(),
            refresh_token_hash: Secret::new(
                "deadbeefcafe0123456789abcdef7890deadbeefcafe0123456789abcdef7890".to_string(),
            ),
            provider: "google".to_string(),
            expires_at: now + chrono::Duration::hours(1),
            device_id: Some("device-1".to_string()),
            user_agent: Some("ua/1".to_string()),
            ip_address: Some("192.0.2.9".to_string()),
            created_at: now,
        }
    }

    const HASH_SENTINEL: &str = "deadbeefcafe0123456789abcdef7890deadbeefcafe0123456789abcdef7890";

    /// The hand-written Debug redacts exactly the hash and nothing else.
    #[test]
    fn debug_output_redacts_only_the_refresh_token_hash() {
        let rendered = format!("{:?}", sample_session());

        assert!(
            rendered.contains("<redacted>"),
            "debug output must show the redaction marker"
        );
        assert!(
            !rendered.contains(HASH_SENTINEL),
            "debug output must never contain the refresh-token hash"
        );
        // Non-sensitive fields stay observable for operators.
        assert!(rendered.contains("usr_debug_test"));
        assert!(rendered.contains("google"));
        assert!(rendered.contains("device-1"));
    }

    /// serde transparency: the stored/JSON shape is byte-identical to a plain-string
    /// session, so every store keeps writing and reading the same 64-hex string with no
    /// migration.
    #[test]
    fn json_round_trip_is_string_identical_to_a_plain_string_shape() {
        let session = sample_session();

        let serialized = serde_json::to_string(&session).expect("serialize session");
        assert!(
            !serialized.contains('<'),
            "serialization must produce plain JSON strings only",
        );

        let expected = serde_json::json!({
            "user_id": session.user_id,
            "refresh_token_hash": HASH_SENTINEL,
            "provider": session.provider,
            "expires_at": session.expires_at.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true),
            "device_id": session.device_id,
            "user_agent": session.user_agent,
            "ip_address": session.ip_address,
            "created_at": session.created_at.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true),
        })
        .to_string();
        // Compare structurally rather than lexically: field order may differ, values must
        // not. The critical property under test is that refresh_token_hash serializes as
        // the bare hex string, which the equality below pins down.
        let left: serde_json::Value =
            serde_json::from_str(&serialized).expect("serialized session is valid JSON");
        let right: serde_json::Value =
            serde_json::from_str(&expected).expect("expected shape is valid JSON");
        assert_eq!(left["refresh_token_hash"], right["refresh_token_hash"]);

        let back: Session = serde_json::from_str(&serialized).expect("deserialize session");
        assert_eq!(back.refresh_token_hash.expose(), &HASH_SENTINEL.to_string());
        assert_eq!(back.user_id, session.user_id);
        assert_eq!(back.device_id, session.device_id);
    }
}
