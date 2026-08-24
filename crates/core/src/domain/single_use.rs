/// A presence-only single-use record backing the direct ID-token grant's replay
/// protection: nonces minted for the grant and assertion-replay markers are both stored
/// as a namespaced digest key plus an expiry, and nothing else. The key is all the
/// information there is — nonce values and assertions reach storage only as SHA-256 hex
/// digests (as refresh tokens already do), never in raw form.
///
/// Records are removed by [`crate::ports::SessionRepository::take_single_use`], by
/// store-native expiry (DynamoDB TTL / Valkey `SET EX`), or by the
/// [`crate::ports::SessionRepository::cleanup_expired_sessions`] sweep where native
/// expiry does not exist.
///
/// See `01-domain-model.md` → Entities → SingleUseRecord and
/// `08-persistence.md` → Single-use records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SingleUseRecord {
    /// Namespaced digest key: `"nonce:<sha256hex>"` or
    /// `"assertion:<provider>:[d:]<sha256hex>"`. Never a raw nonce or raw assertion.
    pub key: String,
    /// Instant after which the record is treated as absent by both claim operations.
    pub expires_at: DateTime<Utc>,
}

use chrono::{DateTime, Utc};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_round_trips_fields() {
        let expires_at = Utc::now() + chrono::Duration::minutes(10);
        let record = SingleUseRecord {
            key: "nonce:abc123".to_string(),
            expires_at,
        };

        assert_eq!(record.key, "nonce:abc123");
        assert_eq!(record.expires_at, expires_at);
        // Structural equality keys on both fields: two records with different expiries
        // are different records even at the same key.
        let later = SingleUseRecord {
            key: record.key.clone(),
            expires_at: expires_at + chrono::Duration::seconds(1),
        };
        assert_ne!(record, later);
    }
}
