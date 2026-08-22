//! Safe extraction of upstream error detail for provider-facing errors.
//!
//! VENDORED PREREQUISITE — owned by sibling change
//! `.specs/changes/2026-08-05-eliminate_secret_leakage_in_logs_and_spans.md`,
// which specifies `upstream::error_detail` and routes the error-body call sites
//! through it. That sibling change is not merged into this unstacked branch, so
//! the outbound-boundary work pins the exact contract locally. The owning PR
//! reconciles ownership: delete this copy or repoint imports at the sibling's
//! helper. Nothing here widens the sibling's contract.
//!
//! The security property this module owns: an upstream failure may name *what*
//! went wrong in protocol terms, but never echoes response bodies, secrets, or
//! arbitrary upstream-controlled text into logs. Error surfaces stay generic.

/// Hard bound on any single upstream-supplied fragment embedded in an error
/// detail string, in bytes. OAuth `error` / `error_description` values are
/// short protocol tokens; anything longer is not worth surfacing and must not
/// become a log-injection channel.
const MAX_ERROR_FRAGMENT_BYTES: usize = 256;

/// Marker appended when a fragment was cut at [`MAX_ERROR_FRAGMENT_BYTES`].
const TRUNCATED_MARKER: &str = "…[truncated]";

/// Build the safe `detail` for a non-success upstream response.
///
/// When the (already bounded) body is a JSON document carrying the OAuth
/// protocol fields, the `error` code — and `error_description` when present —
/// are surfaced because they are bounded, enumerable protocol tokens operators
/// need to act on. Every other shape collapses to a generic message naming the
/// HTTP status: the raw body is never echoed, so hostile or buggy upstreams
/// cannot write into our logs through their responses.
pub fn error_detail(status: reqwest::StatusCode, body: &[u8]) -> String {
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) {
        if let Some(object) = value.as_object() {
            if let Some(error) = object.get("error").and_then(|v| v.as_str()) {
                return match object.get("error_description").and_then(|v| v.as_str()) {
                    Some(description) => format!(
                        "{}: {}",
                        bounded_fragment(error),
                        bounded_fragment(description)
                    ),
                    None => bounded_fragment(error),
                };
            }
        }
    }

    format!("upstream returned HTTP {status}")
}

/// Bound one upstream-supplied fragment to [`MAX_ERROR_FRAGMENT_BYTES`],
/// cutting on a char boundary and marking the cut.
fn bounded_fragment(fragment: &str) -> String {
    if fragment.len() <= MAX_ERROR_FRAGMENT_BYTES {
        return fragment.to_string();
    }

    let mut end = MAX_ERROR_FRAGMENT_BYTES;
    while end > 0 && !fragment.is_char_boundary(end) {
        end -= 1;
    }
    assert!(
        end > 0,
        "MAX_ERROR_FRAGMENT_BYTES must exceed any single UTF-8 character's length"
    );

    let mut bounded = fragment[..end].to_string();
    bounded.push_str(TRUNCATED_MARKER);
    bounded
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(code: u16) -> reqwest::StatusCode {
        reqwest::StatusCode::from_u16(code).expect("test status codes are valid")
    }

    #[test]
    fn oauth_error_and_description_are_surfaced_for_operators() {
        let body = br#"{"error":"invalid_grant","error_description":"code expired"}"#;
        let detail = error_detail(status(400), body);

        assert_eq!(detail, "invalid_grant: code expired");
        assert!(
            detail.contains("invalid_grant") && detail.contains("code expired"),
            "protocol tokens must survive: {detail}"
        );
    }

    #[test]
    fn oauth_error_without_description_stands_alone() {
        let body = br#"{"error":"server_error"}"#;
        let detail = error_detail(status(500), body);

        assert_eq!(detail, "server_error");
        assert!(!detail.contains('{'), "no raw JSON braces leak: {detail}");
    }

    #[test]
    fn non_json_body_collapses_to_a_generic_status_message() {
        // Binary junk that is also invalid UTF-8: nothing about it may reach the log.
        let body: &[u8] = &[
            0xFF, 0xFE, 0x00, b'<', b's', b'c', b'r', b'i', b'p', b't', b'>',
        ];
        let detail = error_detail(status(502), body);

        assert_eq!(detail, "upstream returned HTTP 502 Bad Gateway");
        assert!(
            !detail.contains("script"),
            "the body must never be echoed into the detail: {detail}"
        );
    }

    #[test]
    fn json_without_an_error_field_is_equally_generic() {
        // A JSON body that is not an OAuth error document gets no special trust:
        // only the enumerated protocol fields are ever surfaced.
        let body = br#"{"secret":"hunter2","stack_trace":"file.rs:1"}"#;
        let detail = error_detail(status(500), body);

        assert_eq!(detail, "upstream returned HTTP 500 Internal Server Error");
        assert!(
            !detail.contains("hunter2") && !detail.contains("stack_trace"),
            "non-protocol JSON content must never leak: {detail}"
        );
    }

    #[test]
    fn oversized_fragments_are_truncated_at_the_named_bound() {
        let long_description = "d".repeat(MAX_ERROR_FRAGMENT_BYTES + 500);
        let body = serde_json::json!({
            "error": "invalid_grant",
            "error_description": long_description,
        })
        .to_string()
        .into_bytes();

        let detail = error_detail(status(400), &body);

        assert!(
            detail.ends_with(TRUNCATED_MARKER),
            "a cut fragment must be marked as truncated: {detail}"
        );
        // error + ": " + description + marker bounds the whole string; the
        // description itself can contribute no more than its bound plus marker.
        let description_portion = detail
            .strip_prefix("invalid_grant: ")
            .expect("error code should lead the detail");
        assert!(
            description_portion.len() <= MAX_ERROR_FRAGMENT_BYTES + TRUNCATED_MARKER.len(),
            "fragment must be bounded: {} bytes",
            description_portion.len()
        );
    }

    #[test]
    fn truncation_cuts_on_a_char_boundary_not_mid_codepoint() {
        // A multi-byte character straddling the cut point must not panic or
        // produce invalid UTF-8 when sliced.
        let multibyte = "é".repeat(MAX_ERROR_FRAGMENT_BYTES); // 2 bytes each
        let body = serde_json::json!({ "error": multibyte })
            .to_string()
            .into_bytes();

        let detail = error_detail(status(400), &body);
        assert!(
            detail.contains(TRUNCATED_MARKER),
            "an oversized multibyte fragment is still bounded and marked: {detail}"
        );
    }
}
