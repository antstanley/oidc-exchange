# Task 04 — Add bounded, redacting upstream-error helper

**Status:** Done · **Plan:** [plan.md](../plan.md) · **Certificate:** forbidden and intentionally omitted

**Implements:** [source spec §02-ports-and-adapters — Shared OIDC utilities](../../../changes/2026-08-05-eliminate_secret_leakage_in_logs_and_spans.md#specsservicespecs02-ports-and-adaptersmd--shared-oidc-utilities-modify); [§Implementation notes step 3](../../../changes/2026-08-05-eliminate_secret_leakage_in_logs_and_spans.md#implementation-notes); [`02-ports-and-adapters.md` Shared OIDC utilities](../../../service/specs/02-ports-and-adapters.md)
**Depends on:** 03
**Produces:** shared bounded HTTP body read returning `Secret<String>` and a single `upstream::error_detail(status, body)` constructor that produces bounded, percent-decoded redacted diagnostics.
**Pointers:** `crates/adapters/src/shared/http.rs`, new `shared/upstream.rs`, `shared/mod.rs`, `shared/token_endpoint.rs`; adapter test utilities/wiremock fixtures.

## Steps

- [x] Add `MAX_UPSTREAM_BODY_BYTES: usize = 65_536` and stream a response body to that ceiling rather than calling unbounded `text()`; return `Secret<String>` and define explicit behavior for read/truncation failure.
- [x] Add `MAX_UPSTREAM_EXCERPT: usize = 256` and `upstream::error_detail(StatusCode, Secret<String>) -> String` as the only shared upstream-body-to-detail path.
- [x] Prefer RFC 6749 JSON `error` and optional `error_description`; otherwise produce status, original byte length, and only a bounded redacted excerpt.
- [x] Percent-decode before masking form keys `token`, `refresh_token`, `client_secret`, and `code`, and redact bare compact JWS values; ensure malformed encoding cannot bypass masking or panic.
- [x] Export the helper and reader from `shared/mod.rs`; add focused unit/wiremock tests for structured OAuth preservation, encoded echoed form values, JWS, malformed input, oversized bodies, and excerpt boundaries.

## Task-specific definition of done

- [x] No upstream body is fully buffered beyond 64 KiB, and no function returns a format-capable raw body before the reviewed conversion point.
- [x] Every fallback diagnostic is bounded to 256 characters after redaction and contains no supplied secret sentinels, including percent-encoded forms.
- [x] Conformant OAuth `error`/`error_description` behavior remains available for task 05’s existing token-endpoint regression.
- [x] No certificate file is created; test output is the completion evidence.

**Evidence:** commit `feat(adapters)` on this workspace (task 04). New `shared/upstream.rs`: `error_detail` (asserts non-2xx precondition; consumes the `Secret`; prefers the RFC 6749 object with both fields passed through the same redact+clamp pipeline; fallback renders status + original byte length + ≤256-char redacted excerpt with a postcondition assert) and the redactor (percent-decode-first with malformed escapes copied through literally, form-pair masking anywhere in text behind a delimiter boundary so `token_type_hint=`/`grant_type=…code` are untouched, JSON `"key":"value"` masking, bare compact-JWS masking gated on a base64url header decoding to `{`). `http::read_bounded(provider, response)` streams `bytes_stream()` under `MAX_UPSTREAM_BODY_BYTES`, truncates at the ceiling with a structured body-free `warn!`, converts non-UTF-8 lossily, and maps stream failures to a `ProviderError` carrying only transport text. Both exported from `shared/mod.rs`. Tests (all passing): structured preservation with/without description, structured-field redaction+bounding, status/length/excerpt fallback, percent-encoded echo, five malformed-escape shapes, JSON-style echo, bare JWS masked vs dotted-text not falsely masked, excerpt clamped exactly at 256 / one-under unclamped, empty body, char-boundary clamping, refresh_token-vs-token_type_hint prefix rule, code sentinel; wiremock: small round-trip, exactly-at-ceiling kept, one-byte-over truncated to ceiling, invalid UTF-8 lossy, oversize emits exactly one structured warn naming provider+limit. Focused: `cargo nextest run -p oidc-exchange-adapters` — 149 passed, 28 skipped ([ignored] integration tier); adapters clippy `-D warnings` clean.
