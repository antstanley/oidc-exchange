use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Prefix every session-family identifier carries (`fam_` + lowercase ULID).
/// See `is_valid_family_id` for the exact accepted form.
pub const FAMILY_ID_PREFIX: &str = "fam_";

/// Number of characters in a ULID's canonical string form. A well-formed
/// family id is [`FAMILY_ID_PREFIX`] followed by exactly this many lowercase
/// Crockford-base32 characters.
pub const ULID_CHAR_LEN: usize = 26;

/// The lowercase Crockford-base32 alphabet ULIDs render in (`ulid`'s string
/// form, lowercased). Digits plus `abcdefghjkmnpqrstvwxyz` — `i`, `l`, `o`,
/// and `u` are excluded to avoid human-transcription ambiguity.
const ULID_LOWERCASE_ALPHABET: &str = "0123456789abcdefghjkmnpqrstvwxyz";

/// Whether `id` is a well-formed session-family identifier:
/// [`FAMILY_ID_PREFIX`] + exactly [`ULID_CHAR_LEN`] lowercase
/// Crockford-base32 characters (e.g. `fam_01j2m3n4p5q6r7s8t9v0wxyzab`).
///
/// Family ids are minted by core ([`new_family_id`]) and validated at the
/// boundaries where they re-enter from outside — a malformed id can never name
/// an existing family, so accepting one silently would only mask a caller bug.
pub fn is_valid_family_id(id: &str) -> bool {
    let Some(ulid_part) = id.strip_prefix(FAMILY_ID_PREFIX) else {
        return false;
    };
    if ulid_part.len() != ULID_CHAR_LEN {
        return false;
    }
    ulid_part
        .chars()
        .all(|c| ULID_LOWERCASE_ALPHABET.contains(c))
}

/// Mint a fresh, well-formed session-family identifier: [`FAMILY_ID_PREFIX`]
/// plus a lowercase ULID. Core generates one per token exchange; the value is
/// stable across every rotation of that sign-in.
pub fn new_family_id() -> String {
    format!(
        "{}{}",
        FAMILY_ID_PREFIX,
        ulid::Ulid::new().to_string().to_lowercase()
    )
}

/// One generation of a refresh-token family.
///
/// The raw refresh token exists only in memory during issuance and in the
/// response to the client; only the hash is stored. `family_id` and
/// `created_at` identify the sign-in and survive rotation; `expires_at` is the
/// family's absolute deadline and is copied unchanged into every replacement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub user_id: String,
    /// SHA-256 hex of the opaque token; never the raw token itself.
    pub refresh_token_hash: String,
    /// `fam_<lowercase ULID>`; stable across every rotation of one sign-in.
    pub family_id: String,
    /// 0 at exchange, incremented once per rotation. Exactly one generation of
    /// a family is live at any instant.
    pub generation: u32,
    pub provider: String,
    /// Absolute family deadline; set at exchange and never moved by rotation.
    pub expires_at: DateTime<Utc>,
    /// When this generation was issued; `None` at generation 0.
    pub rotated_at: Option<DateTime<Utc>>,
    pub device_id: Option<String>,
    pub user_agent: Option<String>,
    pub ip_address: Option<String>,
    /// When the family was created (not when this generation was issued).
    pub created_at: DateTime<Utc>,
}

/// A retired refresh-token generation, retained so that its re-presentation is
/// detectable as reuse.
///
/// It is written by the same atomic store operation that retires the
/// generation it names, and it expires at
/// `min(retired_at + refresh_reuse_retention, family expires_at)` — after that
/// a presented generation resolves as unknown rather than as reuse.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetiredRefreshToken {
    /// SHA-256 hex of the retired generation.
    pub refresh_token_hash: String,
    /// The family this generation belongs to.
    pub family_id: String,
    /// The user this generation belonged to.
    pub user_id: String,
    /// SHA-256 hex of the generation that replaced this one. Grace applies
    /// only while this successor is still the family's live generation.
    pub successor_hash: String,
    /// When the retirement happened (the instant of its replacing rotation).
    pub retired_at: DateTime<Utc>,
    /// `min(retired_at + refresh_reuse_retention, family expires_at)`; past
    /// this instant the record may be swept like any expired row.
    pub expires_at: DateTime<Utc>,
}

/// How a presented refresh-token hash classifies against the store's family
/// state — the value [`crate::ports::SessionRepository::resolve_refresh_token`]
/// returns.
///
/// This is a storage fact, not a policy decision: whether a [`RefreshResolution::Superseded`]
/// hash may still rotate (the grace window) or must be treated as reuse is
/// evaluated once in the core service against configuration, never in the
/// adapters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshResolution {
    /// The hash is a family's currently-live generation.
    Live(Session),
    /// The hash is retired, and the successor it names is still that family's
    /// live generation. `live` is that successor session; `retired_at` is when
    /// the presented hash was retired.
    Superseded {
        live: Session,
        retired_at: DateTime<Utc>,
    },
    /// The hash is retired and its successor is no longer live — presenting it
    /// is reuse of an already-superseded credential.
    Retired {
        family_id: String,
        user_id: String,
        retired_at: DateTime<Utc>,
    },
    /// No live generation and no retained retirement record matches the hash.
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_well_formed_lowercase_ulid_family_ids() {
        // 26 lowercase Crockford-base32 digits after the prefix.
        assert!(is_valid_family_id("fam_01j2m3n4p5q6r7s8t9v0wxyzab"));
        assert!(is_valid_family_id("fam_00000000000000000000000000"));
        assert!(is_valid_family_id("fam_zzzzzzzzzzzzzzzzzzzzzzzzzz"));
    }

    #[test]
    fn rejects_malformed_family_ids() {
        // Missing / wrong prefix.
        assert!(!is_valid_family_id("01j2m3n4p5q6r7s8t9v0wxyzab"));
        assert!(!is_valid_family_id(""));
        // Uppercase is not the canonical form.
        assert!(!is_valid_family_id("FAM_01J2M3N4P5Q6R7S8T9V0WXYZAB"));
        assert!(!is_valid_family_id("fam_01J2M3N4P5Q6R7S8T9V0WXYZAB"));
        // Wrong length: one short, one long, and a SHA-256-hex-valued sid
        // (the pre-rotation form later waves must keep rejecting).
        assert!(!is_valid_family_id("fam_01j2m3n4p5q6r7s8t9v0wxyza"));
        assert!(!is_valid_family_id("fam_01j2m3n4p5q6r7s8t9v0wxyzabc"));
        assert!(!is_valid_family_id(&format!(
            "fam_{}",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        )));
        // Characters outside the Crockford alphabet ('i', 'l', 'o', 'u'),
        // padded to the exact expected length so only the alphabet fails.
        for bad in ['i', 'l', 'o', 'u'] {
            let candidate = format!("fam_{}", bad.to_string().repeat(ULID_CHAR_LEN));
            assert!(
                !is_valid_family_id(&candidate),
                "{candidate} must be rejected"
            );
        }
    }

    #[test]
    fn new_family_ids_are_valid_and_unique() {
        let first = new_family_id();
        let second = new_family_id();

        assert!(is_valid_family_id(&first), "{first} must be well-formed");
        assert!(is_valid_family_id(&second), "{second} must be well-formed");
        assert_ne!(first, second, "ULIDs must not repeat");
        assert_eq!(first.len(), FAMILY_ID_PREFIX.len() + ULID_CHAR_LEN);
        assert!(first.starts_with(FAMILY_ID_PREFIX));
        assert!(
            first
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()),
            "family ids must be entirely lowercase: {first}"
        );
    }
}
