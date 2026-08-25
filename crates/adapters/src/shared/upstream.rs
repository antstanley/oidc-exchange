//! The single audited path from upstream response bytes to a loggable error detail.
//!
//! [`error_detail`] is the only constructor a `ProviderError` detail may be built from
//! upstream body text: it consumes the body as a [`Secret<String>`] (produced by
//! [`crate::shared::http::read_bounded`]) and returns a bounded, redacted, plain
//! `String`. Before this module existed, each provider call site interpolated the raw
//! non-2xx body into its own detail string — which the server's error mapping logs in
//! full — so an upstream that echoed a submitted token or client assertion put that
//! credential into the operator log. Redaction therefore happens *here*, once, where it
//! can be audited and tested, instead of at every call site.

use oidc_exchange_core::Secret;

/// Maximum number of characters any excerpt embedded in an [`error_detail`] result may
/// carry. Bounds what a hostile upstream can make the service retain inside an error
/// string (and later render into a log line) regardless of how large its body was.
pub const MAX_UPSTREAM_EXCERPT: usize = 256;

/// Marker substituted for every redacted value. Chosen to be self-describing in a log
/// line without resembling any credential shape.
const REDACTED: &str = "[REDACTED]";

/// Form keys whose echoed values are credentials and must never survive into a detail.
/// Matched case-sensitively: RFC 6749 form parameter names are lowercase.
const SENSITIVE_FORM_KEYS: [&str; 4] = ["token", "refresh_token", "client_secret", "code"];

/// Minimum length of any single compact-JWS segment; excludes degenerate short runs
/// from the bare-JWS mask without making the check content-aware beyond the header.
const MIN_JWS_SEGMENT_LEN: usize = 4;

/// Build the loggable detail for a non-2xx upstream response from its (bounded) body.
///
/// Preference order:
///
/// 1. A structured RFC 6749 error object (`error`, optionally `error_description`) —
///    preserved because conformant OAuth error content is exactly what operators need
///    to see and what callers assert on. Each field still passes through the same
///    redacting pipeline as the fallback excerpt and is clamped to
///    [`MAX_UPSTREAM_EXCERPT`] characters, so a hostile upstream cannot smuggle an
///    echoed credential inside `error_description` either.
/// 2. Otherwise: the HTTP status, the body's byte length, and a redacted excerpt capped
///    at [`MAX_UPSTREAM_EXCERPT`] characters. The length is reported so a truncated or
///    empty excerpt stays distinguishable from a short one.
///
/// This is deliberately the *only* shared function that turns upstream bytes into a
/// plain, format-capable string; keeping one constructor means a fourth copy of the old
/// raw-body pattern has nowhere to come from.
pub fn error_detail(status: reqwest::StatusCode, body: Secret<String>) -> String {
    // Precondition: this constructor is only meaningful for failure responses; calling
    // it on a 2xx body would signal a misrouted success payload.
    assert!(
        !status.is_success(),
        "upstream::error_detail must only be called on non-2xx responses, got {status}"
    );

    // The audited boundary: the secret is consumed here and everything below works on
    // the raw text that will actually be emitted.
    let raw = body.into_inner();

    if let Some(structured) = structured_error_detail(&raw) {
        return structured;
    }

    let redacted = redact(&raw);
    let excerpt = clamp_chars(&redacted, MAX_UPSTREAM_EXCERPT);
    // Postcondition: the excerpt pipeline is what bounds the diagnostic, so verify the
    // invariant where the string becomes loggable rather than trusting callers.
    assert!(
        excerpt.chars().count() <= MAX_UPSTREAM_EXCERPT,
        "excerpt must stay within MAX_UPSTREAM_EXCERPT after redaction"
    );
    format!(
        "HTTP {status}; upstream returned {len} bytes; excerpt: {excerpt}",
        len = raw.len()
    )
}

/// Prefer the RFC 6749 structured error object when the body carries one.
///
/// Returns `None` whenever the body is not JSON with a string `error` member, leaving
/// [`error_detail`] to fall back to the status/length/excerpt form.
fn structured_error_detail(raw: &str) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_str(raw).ok()?;
    let error_code = parsed.get("error")?.as_str()?;

    let code = clamp_chars(&redact(error_code), MAX_UPSTREAM_EXCERPT).to_string();
    let description = parsed
        .get("error_description")
        .and_then(|v| v.as_str())
        .map(|d| clamp_chars(&redact(d), MAX_UPSTREAM_EXCERPT).to_string());

    Some(match description {
        Some(description) => format!("{code}: {description}"),
        None => code,
    })
}

/// Produce the loggable form of upstream text: percent-decode first, then mask the
/// values of sensitive form keys, JSON string values under those keys, and bare compact
/// JWS values.
///
/// Decoding happens before masking because an upstream that echoes a submitted form
/// hands back percent-encoded values (`token=1%2F%2F…`); a matcher running on the
/// encoded text would pass while the leak remained. Malformed escapes (`%ZZ`, a
/// trailing `%2`) are copied through literally — they can neither panic the decoder nor
/// bypass masking, because masking then runs over whatever the decode produced.
fn redact(text: &str) -> String {
    let decoded = percent_decode(text);
    let forms_masked = mask_form_pairs(&decoded);
    let json_masked = mask_json_values(&forms_masked);
    mask_compact_jws(&json_masked)
}

/// Percent-decode `text` (`%XX` byte escapes), copying malformed sequences through
/// unchanged. Byte-level by design: the output feeds key matching, not display, and a
/// lossy pass cannot panic on arbitrary input.
fn percent_decode(text: &str) -> String {
    // Upper bound: every three-byte escape collapses to one byte, so decoding never
    // grows the text beyond its input length.
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if let (Some(&hi), Some(&lo)) = (bytes.get(i + 1), bytes.get(i + 2)) {
                if let (Some(h), Some(l)) = (hex_digit(hi), hex_digit(lo)) {
                    out.push((h << 4) | l);
                    i += 3;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// One hex digit to its value, or `None` for anything else.
fn hex_digit(byte: u8) -> Option<u8> {
    (byte as char).to_digit(16).map(|v| v as u8)
}

/// Mask the value of every `key=value` pair whose key is in [`SENSITIVE_FORM_KEYS`],
/// wherever the pair occurs in the text — an upstream may echo a submitted form back as
/// a bare query string, embedded in prose, or inside an HTML page. The key must sit on
/// a delimiter boundary (start of text, `&`, whitespace, punctuation) and be followed
/// directly by `=`, so `token_type_hint=` is not mistaken for `token=` and the `code`
/// inside `grant_type=authorization_code` stays untouched. Values run to the next `&`
/// or the end of the text; over-masking past a prose sentence end is the safe
/// direction.
fn mask_form_pairs(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        // Earliest matching pair wins so rewrites stay in document order.
        let mut earliest: Option<(usize, &str)> = None;
        for key in SENSITIVE_FORM_KEYS {
            if let Some(pos) = find_form_pair(rest, key) {
                if earliest.is_none_or(|(p, _)| pos < p) {
                    earliest = Some((pos, key));
                }
            }
        }
        let Some((pos, key)) = earliest else { break };
        out.push_str(&rest[..pos]);
        out.push_str(key);
        out.push('=');
        out.push_str(REDACTED);
        // Advance past the value: everything from just after `=` to the next `&`.
        let after_eq = pos + key.len() + 1;
        rest = match rest[after_eq..].find('&') {
            Some(offset) => &rest[after_eq + offset..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}

/// Find the first position in `text` where `key` sits on a delimiter boundary and is
/// immediately followed by `=`. Returns the byte offset of the key.
fn find_form_pair(text: &str, key: &str) -> Option<usize> {
    let mut search_from = 0;
    while let Some(rel) = text[search_from..].find(key) {
        let pos = search_from + rel;
        let after_key = pos + key.len();
        // Boundary before: a word character glued to the key means this is a longer
        // identifier (`opentoken`, `my_code`), not the form parameter.
        let boundary_before = match text[..pos].chars().next_back() {
            None => true,
            Some(prev) => !(prev.is_ascii_alphanumeric() || prev == '_'),
        };
        if boundary_before && text[after_key..].starts_with('=') {
            return Some(pos);
        }
        // Keys never overlap themselves; advancing past this occurrence is exhaustive.
        search_from = after_key;
    }
    None
}

/// Mask JSON string values whose key is in [`SENSITIVE_FORM_KEYS`], covering bodies
/// that echo submitted parameters as a JSON object (`"token": "…"`) rather than a form
/// pair. Occurrences are scanned left to right; each rewrite consumes at least the key
/// it matched, so the loop terminates on every input.
fn mask_json_values(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        // Earliest sensitive key wins, so overlapping keys (`token` before
        // `refresh_token`) are handled in document order.
        let mut earliest: Option<(usize, &str)> = None;
        for key in SENSITIVE_FORM_KEYS {
            let needle = format!("\"{key}\"");
            if let Some(pos) = rest.find(&needle) {
                if earliest.is_none_or(|(p, _)| pos < p) {
                    earliest = Some((pos, key));
                }
            }
        }
        let Some((pos, key)) = earliest else { break };
        let after_key = pos + key.len() + 2; // + the two surrounding quotes
        out.push_str(&rest[..after_key]);
        match json_string_value_range(&rest[after_key..]) {
            Some((value_start, value_end)) => {
                // Copy the whitespace/colon/opening quote verbatim, replace only the
                // value, and resume at the closing quote.
                out.push_str(&rest[after_key..after_key + value_start]);
                out.push_str(REDACTED);
                rest = &rest[after_key + value_end..];
            }
            None => {
                // No string value follows this occurrence; keep scanning after it.
                rest = &rest[after_key..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Given the text immediately following a `"key"` member name, locate the range (within
/// that slice) of the JSON string value after `<ws> : <ws> "` … `"`. The end index
/// points at the closing quote so the caller can resume there. Returns `None` when no
/// such value follows, leaving the occurrence untouched rather than guessing.
fn json_string_value_range(after_key: &str) -> Option<(usize, usize)> {
    let mut chars = after_key.char_indices();
    let colon = chars.find(|(_, ch)| !ch.is_whitespace())?;
    if colon.1 != ':' {
        return None;
    }
    let quote = chars.find(|(_, ch)| !ch.is_whitespace())?;
    if quote.1 != '"' {
        return None;
    }
    let value_start = quote.0 + 1;
    // Terminate at the next double quote. An escaped quote inside an echoed secret is
    // exotic; stopping early only narrows the mask, it never panics.
    let closing = after_key[value_start..].find('"')? + value_start;
    Some((value_start, closing))
}

/// Replace bare compact JWS values (`header.payload.signature`, base64url segments)
/// with [`REDACTED`].
///
/// A run counts as a JWS only when it splits into exactly three base64url segments and
/// the first decodes to UTF-8 opening with `{` — the JSON object every JWS header is —
/// which keeps ordinary dotted text (version strings, file names) unmasked. Form-pair
/// masking already covers assertions posted as `client_secret=<jws>`; this pass catches
/// assertions echoed outside any key/value context.
fn mask_compact_jws(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    // `None` while outside a candidate run; `Some(start)` while inside one. Runs are the
    // maximal stretches of the JWS charset, whose separators (`.`) stay inside the run
    // so a whole `a.b.c` candidate is examined at once.
    let mut run_start: Option<usize> = None;
    for (idx, ch) in text.char_indices() {
        let in_charset = ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.');
        if in_charset {
            run_start.get_or_insert(idx);
        } else {
            if let Some(start) = run_start.take() {
                push_jws_checked(&mut out, &text[start..idx]);
            }
            out.push(ch);
        }
    }
    if let Some(start) = run_start {
        push_jws_checked(&mut out, &text[start..]);
    }
    // Postcondition: the rewrite only ever replaces a run with a shorter marker or
    // copies text through, so the output can never exceed the input.
    assert!(
        out.len() <= text.len(),
        "JWS masking must never grow the text"
    );
    out
}

/// Append `candidate` to `out`, substituting [`REDACTED`] when it has the shape and
/// JSON-object header of a compact JWS.
fn push_jws_checked(out: &mut String, candidate: &str) {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine as _;

    let parts: Vec<&str> = candidate.split('.').collect();
    let shape_ok = parts.len() == 3
        && parts.iter().all(|part| {
            part.len() >= MIN_JWS_SEGMENT_LEN
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        });
    if !shape_ok {
        out.push_str(candidate);
        return;
    }
    let header_is_json = URL_SAFE_NO_PAD
        .decode(parts[0])
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .is_some_and(|decoded| decoded.starts_with('{'));
    if header_is_json {
        out.push_str(REDACTED);
    } else {
        out.push_str(candidate);
    }
}

/// Clamp `value` to at most `max_chars` characters, cutting only on char boundaries.
fn clamp_chars(value: &str, max_chars: usize) -> &str {
    match value.char_indices().nth(max_chars) {
        Some((byte_idx, _)) => &value[..byte_idx],
        None => value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Obviously-fake sentinels used across these tests. They are shaped like the real
    /// thing (so the masks have something to catch) but carry no credential material.
    const TOKEN_SENTINEL: &str = "SENTINEL-TOKEN-VALUE";
    const CODE_SENTINEL: &str = "SENTINEL-CODE-VALUE";

    fn detail_for_status(status: reqwest::StatusCode, body: String) -> String {
        error_detail(status, Secret::new(body))
    }

    #[test]
    fn error_detail_requires_non_success_status() {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            detail_for_status(reqwest::StatusCode::OK, "irrelevant".to_string())
        }));
        assert!(
            result.is_err(),
            "calling error_detail on a 2xx status must be a programmer error"
        );
    }

    #[test]
    fn structured_rfc6749_error_is_preferred_with_description() {
        let body =
            r#"{"error":"invalid_grant","error_description":"the authorization code has expired"}"#;
        let detail = detail_for_status(reqwest::StatusCode::BAD_REQUEST, body.to_string());
        assert_eq!(detail, "invalid_grant: the authorization code has expired");
    }

    #[test]
    fn structured_error_without_description_renders_code_alone() {
        let detail = detail_for_status(
            reqwest::StatusCode::BAD_REQUEST,
            r#"{"error":"slow_down"}"#.to_string(),
        );
        assert_eq!(detail, "slow_down");
    }

    #[test]
    fn structured_error_fields_are_redacted_and_bounded() {
        // A hostile upstream echoing the submitted token inside error_description must
        // not smuggle it through the structured path.
        let body = format!(
            r#"{{"error":"invalid_grant","error_description":"rejected token={TOKEN_SENTINEL} {}"}}"#,
            "x".repeat(MAX_UPSTREAM_EXCERPT + 50)
        );
        let detail = detail_for_status(reqwest::StatusCode::BAD_REQUEST, body);
        assert!(
            !detail.contains(TOKEN_SENTINEL),
            "structured description must be redacted, got {detail:?}"
        );
        // Both fields are clamped independently: code + ": " + description.
        assert!(
            detail.chars().count() <= 2 * MAX_UPSTREAM_EXCERPT + 3,
            "structured detail must be bounded, got {} chars",
            detail.chars().count()
        );
    }

    #[test]
    fn non_json_body_reports_status_length_and_redacted_excerpt() {
        let body = format!("upstream exploded while handling token={TOKEN_SENTINEL}");
        let detail = detail_for_status(reqwest::StatusCode::INTERNAL_SERVER_ERROR, body.clone());
        assert!(
            detail.starts_with("HTTP 500 Internal Server Error;"),
            "fallback must lead with the status, got {detail:?}"
        );
        assert!(
            detail.contains(&format!("{} bytes", body.len())),
            "fallback must report the original byte length, got {detail:?}"
        );
        assert!(
            detail.contains("excerpt: "),
            "fallback must mark the excerpt, got {detail:?}"
        );
        assert!(
            !detail.contains(TOKEN_SENTINEL),
            "echoed form token must be redacted, got {detail:?}"
        );
        assert!(
            detail.contains("token=[REDACTED]"),
            "the masked pair should remain legible, got {detail:?}"
        );
    }

    #[test]
    fn percent_encoded_echo_is_decoded_before_masking() {
        // The spec's motivating shape: an upstream echoes the submitted form back with
        // percent-encoded values, so a literal-value matcher would pass while the leak
        // remained.
        let body = "error=invalid_token&token=1%2F%2FSENTINEL-PERCENT-ENCODED&client_id=app";
        let detail = detail_for_status(reqwest::StatusCode::UNAUTHORIZED, body.to_string());
        assert!(
            !detail.contains("SENTINEL-PERCENT-ENCODED"),
            "decoded echo must be masked, got {detail:?}"
        );
        assert!(detail.contains("token=[REDACTED]"), "got {detail:?}");
    }

    #[test]
    fn malformed_percent_escapes_do_not_panic_or_bypass_masking() {
        // Each hostile prefix sits inside the value of a sensitive pair, followed by the
        // sentinel: whatever the broken escapes decode to, the value must be masked.
        for hostile in [
            "token=%ZZ%2",       // non-hex digits and a truncated escape
            "token=%",           // lone percent at end of pair
            "token=%A",          // single hex digit at end
            "code=%E2%82",       // cut multi-byte UTF-8 escape sequence
            "refresh_token=%%%", // nothing but broken escapes
        ] {
            let detail = detail_for_status(
                reqwest::StatusCode::BAD_REQUEST,
                format!("{hostile}{TOKEN_SENTINEL}"),
            );
            assert!(!detail.contains(TOKEN_SENTINEL), "{hostile}: {detail:?}");
            assert!(
                detail.contains(REDACTED),
                "{hostile}: the pair must be masked despite malformed escapes, \
                 got {detail:?}"
            );
        }
    }

    #[test]
    fn json_echo_of_sensitive_keys_is_masked_in_free_text() {
        // Not a top-level RFC 6749 object, so the fallback excerpt path runs — and the
        // JSON-style echo inside it must still be masked.
        let body = format!(r#"rejected payload {{"token":"{TOKEN_SENTINEL}"}} retry"#);
        let detail = detail_for_status(reqwest::StatusCode::BAD_REQUEST, body);
        assert!(
            !detail.contains(TOKEN_SENTINEL),
            "JSON-echoed token must be masked, got {detail:?}"
        );
        assert!(detail.contains(r#""token":"[REDACTED]""#), "got {detail:?}");
    }

    #[test]
    fn bare_compact_jws_is_redacted() {
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        use base64::Engine as _;

        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"ES256","kid":"k1"}"#);
        let payload = URL_SAFE_NO_PAD.encode(br#"{"iss":"team"}"#);
        let signature = "c2lnbmF0dXJlLXBsYWNlaG9sZGVyLXNlbnRpbmVs";
        let jws = format!("{header}.{payload}.{signature}");

        // Outside any form context — a plain-text echo.
        let detail = detail_for_status(
            reqwest::StatusCode::FORBIDDEN,
            format!("assertion rejected near {jws} please retry"),
        );
        assert!(
            !detail.contains(signature),
            "bare compact JWS must be redacted, got {detail:?}"
        );
        assert!(detail.contains(REDACTED), "got {detail:?}");
    }

    #[test]
    fn dotted_non_jwt_text_is_not_falsely_masked() {
        let detail = detail_for_status(
            reqwest::StatusCode::BAD_REQUEST,
            "file backup-2026.tar.gz version 1.2.3 failed".to_string(),
        );
        assert!(
            detail.contains("backup-2026.tar.gz"),
            "ordinary dotted text must survive, got {detail:?}"
        );
        assert!(
            detail.contains("1.2.3"),
            "version numbers must survive, got {detail:?}"
        );
    }

    #[test]
    fn excerpt_is_clamped_exactly_at_the_named_bound() {
        // Long benign filler: the only property under test is the character bound.
        let long = "filler ".repeat(200);
        let detail = detail_for_status(reqwest::StatusCode::BAD_GATEWAY, long);

        let excerpt = detail
            .rsplit_once("excerpt: ")
            .expect("fallback always marks its excerpt")
            .1;
        assert_eq!(
            excerpt.chars().count(),
            MAX_UPSTREAM_EXCERPT,
            "excerpt must be clamped exactly at the named constant"
        );
    }

    #[test]
    fn excerpt_one_char_under_the_bound_is_not_clamped() {
        let filler = "f".repeat(MAX_UPSTREAM_EXCERPT - 1);
        let detail = detail_for_status(reqwest::StatusCode::BAD_REQUEST, filler.clone());
        let excerpt = detail.rsplit_once("excerpt: ").unwrap().1;
        assert_eq!(excerpt.chars().count(), MAX_UPSTREAM_EXCERPT - 1);
        assert!(excerpt.starts_with(&filler), "got {excerpt:?}");
    }

    #[test]
    fn empty_body_produces_zero_length_fallback() {
        let detail = detail_for_status(reqwest::StatusCode::NOT_FOUND, String::new());
        assert!(
            detail.contains("upstream returned 0 bytes"),
            "empty body must report its zero length, got {detail:?}"
        );
    }

    #[test]
    fn clamp_chars_cuts_only_on_char_boundaries() {
        let value = "é".repeat(10); // two bytes per char
        assert_eq!(clamp_chars(&value, 4).chars().count(), 4);
        assert_eq!(clamp_chars("abc", 10), "abc");
        assert_eq!(clamp_chars("", 5), "");
        // Exactly at the boundary: no truncation.
        assert_eq!(clamp_chars("abcd", 4), "abcd");
    }

    #[test]
    fn refresh_token_pair_is_masked_but_token_type_hint_is_not() {
        let detail = detail_for_status(
            reqwest::StatusCode::BAD_REQUEST,
            "grant_type=authorization_code&refresh_token=SOME-REFRESH-SENTINEL\
             &token_type_hint=refresh_token"
                .to_string(),
        );
        assert!(
            !detail.contains("SOME-REFRESH-SENTINEL"),
            "refresh_token value must be masked, got {detail:?}"
        );
        assert!(
            detail.contains("token_type_hint=refresh_token"),
            "token_type_hint is not a secret and must stay legible, got {detail:?}"
        );
        // Negative space for the prefix rule: the `code` inside
        // `grant_type=authorization_code` is a value, not a key, and must not be
        // rewritten.
        assert!(
            detail.contains("grant_type=authorization_code"),
            "got {detail:?}"
        );
    }

    #[test]
    fn code_sentinel_is_masked_wherever_it_is_submitted() {
        let detail = detail_for_status(
            reqwest::StatusCode::BAD_REQUEST,
            format!("error=denied&code={CODE_SENTINEL}&client_id=x"),
        );
        assert!(
            !detail.contains(CODE_SENTINEL),
            "submitted code must be masked, got {detail:?}"
        );
    }
}
