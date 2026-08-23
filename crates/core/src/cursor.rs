//! Opaque cursor codecs for the admin plane's bounded reads.
//!
//! A cursor is opaque to the caller and only meaningful to the adapter class
//! that issued it (02-ports-and-adapters.md): this module carries the
//! `(created_at, id)` keyset codec the SQL adapters order by, and which the
//! test-suite mock mirrors so mock-backed tests exercise the same traversal
//! semantics as a durable SQL backend. The DynamoDB adapter keeps its own
//! item-key codec — its cursor is the scan's `LastEvaluatedKey`, a different
//! shape entirely.
//!
//! Cursors are base64url (unpadded) over a compact JSON object, so they
//! survive `encodeURIComponent` round-trips and never carry structural
//! characters that could confuse a query string.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// The last-seen row position a keyset page resumes from: everything
/// strictly after this point in the adapter's total ordering forms the next
/// page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeysetCursor {
    pub created_at: DateTime<Utc>,
    pub id: String,
}

#[derive(Serialize, Deserialize)]
struct KeysetCursorWire {
    /// RFC 3339 rendering of [`KeysetCursor::created_at`].
    c: String,
    /// The row's unique id tiebreaker.
    i: String,
}

impl KeysetCursor {
    pub fn new(created_at: DateTime<Utc>, id: impl Into<String>) -> Self {
        Self {
            created_at,
            id: id.into(),
        }
    }

    /// Render the cursor as its opaque wire form.
    ///
    /// Assertions pin the two properties callers rely on: the output is
    /// non-empty (an empty cursor would be indistinguishable from "no
    /// cursor") and survives an encode/decode round-trip verbatim.
    pub fn encode(&self) -> String {
        assert!(
            !self.id.is_empty(),
            "a keyset cursor must carry a non-empty id: without it the resume position is undefined"
        );

        let wire = KeysetCursorWire {
            c: self.created_at.to_rfc3339(),
            i: self.id.clone(),
        };
        let json = serde_json::to_string(&wire).expect("KeysetCursorWire always serializes");
        let encoded = URL_SAFE_NO_PAD.encode(json.as_bytes());
        assert!(
            !encoded.is_empty(),
            "encoding a valid keyset cursor must never produce an empty string"
        );
        let decoded = Self::decode(&encoded).expect("encode output must decode cleanly");
        assert_eq!(decoded, *self, "cursor encoding must round-trip losslessly");
        encoded
    }

    /// Parse an opaque cursor handed back by a caller.
    ///
    /// Any structural problem — not base64url, wrong JSON shape, missing or
    /// empty id, unparseable timestamp — is [`Error::InvalidRequest`]: a
    /// tampered or stale cursor is a caller fault, never a store error.
    pub fn decode(raw: &str) -> Result<Self> {
        if raw.is_empty() {
            return Err(Error::InvalidRequest {
                reason: "cursor must be a non-empty opaque token".to_string(),
            });
        }

        let json_bytes =
            URL_SAFE_NO_PAD
                .decode(raw.as_bytes())
                .map_err(|_| Error::InvalidRequest {
                    reason: "cursor is not a valid opaque token".to_string(),
                })?;
        let wire: KeysetCursorWire =
            serde_json::from_slice(&json_bytes).map_err(|_| Error::InvalidRequest {
                reason: "cursor is not a valid opaque token".to_string(),
            })?;
        if wire.i.is_empty() {
            return Err(Error::InvalidRequest {
                reason: "cursor is not a valid opaque token".to_string(),
            });
        }
        let created_at = wire
            .c
            .parse::<DateTime<Utc>>()
            .map_err(|_| Error::InvalidRequest {
                reason: "cursor is not a valid opaque token".to_string(),
            })?;
        Ok(Self {
            created_at,
            id: wire.i,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_cursor() -> KeysetCursor {
        KeysetCursor::new(
            "2026-08-05T12:34:56.789Z".parse::<DateTime<Utc>>().unwrap(),
            "usr_01abc",
        )
    }

    #[test]
    fn encode_decode_round_trips() {
        let cursor = sample_cursor();
        let encoded = cursor.encode();
        // URL-safe: no padding, no structural query characters — cursors ride
        // inside `encodeURIComponent`-safe query parameters.
        assert!(!encoded.contains('='));
        assert!(!encoded.contains('&'));
        assert!(!encoded.contains('?'));
        let decoded = KeysetCursor::decode(&encoded).expect("own output must decode");
        assert_eq!(decoded.created_at, cursor.created_at);
        assert_eq!(decoded.id, cursor.id);
    }

    #[test]
    fn decode_rejects_structural_garbage() {
        for bad in [
            "",
            "not-base64!!",
            // Valid base64url of JSON that has the wrong shape.
            &URL_SAFE_NO_PAD.encode(br#"{"nope": 1}"#),
            // Valid base64url of a cursor with an empty id.
            &URL_SAFE_NO_PAD.encode(br#"{"c":"2026-08-05T00:00:00Z","i":""}"#),
            // Valid base64url with an unparseable timestamp.
            &URL_SAFE_NO_PAD.encode(br#"{"c":"yesterday","i":"usr_1"}"#),
        ] {
            let result = KeysetCursor::decode(bad);
            assert!(result.is_err(), "expected {bad:?} to be rejected");
            match result.unwrap_err() {
                Error::InvalidRequest { reason } => {
                    assert!(
                        !reason.is_empty(),
                        "the rejection must carry a reason for {bad:?}"
                    );
                }
                other => panic!("expected InvalidRequest for {bad:?}, got {other:?}"),
            }
        }
    }

    #[test]
    fn encode_rejects_empty_id() {
        let cursor = KeysetCursor::new(Utc::now(), "");
        let result = std::panic::catch_unwind(|| cursor.encode());
        assert!(result.is_err(), "an empty-id cursor is a programmer error");
    }
}
