# Task 04 — Add bounded, redacting upstream-error helper

**Status:** Backlog · **Plan:** [plan.md](../plan.md) · **Certificate:** forbidden and intentionally omitted

**Implements:** [source spec §02-ports-and-adapters — Shared OIDC utilities](../../../changes/2026-08-05-eliminate_secret_leakage_in_logs_and_spans.md#specsservicespecs02-ports-and-adaptersmd--shared-oidc-utilities-modify); [§Implementation notes step 3](../../../changes/2026-08-05-eliminate_secret_leakage_in_logs_and_spans.md#implementation-notes); [`02-ports-and-adapters.md` Shared OIDC utilities](../../../service/specs/02-ports-and-adapters.md)
**Depends on:** 03
**Produces:** shared bounded HTTP body read returning `Secret<String>` and a single `upstream::error_detail(status, body)` constructor that produces bounded, percent-decoded redacted diagnostics.
**Pointers:** `crates/adapters/src/shared/http.rs`, new `shared/upstream.rs`, `shared/mod.rs`, `shared/token_endpoint.rs`; adapter test utilities/wiremock fixtures.

## Steps

- [ ] Add `MAX_UPSTREAM_BODY_BYTES: usize = 65_536` and stream a response body to that ceiling rather than calling unbounded `text()`; return `Secret<String>` and define explicit behavior for read/truncation failure.
- [ ] Add `MAX_UPSTREAM_EXCERPT: usize = 256` and `upstream::error_detail(StatusCode, Secret<String>) -> String` as the only shared upstream-body-to-detail path.
- [ ] Prefer RFC 6749 JSON `error` and optional `error_description`; otherwise produce status, original byte length, and only a bounded redacted excerpt.
- [ ] Percent-decode before masking form keys `token`, `refresh_token`, `client_secret`, and `code`, and redact bare compact JWS values; ensure malformed encoding cannot bypass masking or panic.
- [ ] Export the helper and reader from `shared/mod.rs`; add focused unit/wiremock tests for structured OAuth preservation, encoded echoed form values, JWS, malformed input, oversized bodies, and excerpt boundaries.

## Task-specific definition of done

- [ ] No upstream body is fully buffered beyond 64 KiB, and no function returns a format-capable raw body before the reviewed conversion point.
- [ ] Every fallback diagnostic is bounded to 256 characters after redaction and contains no supplied secret sentinels, including percent-encoded forms.
- [ ] Conformant OAuth `error`/`error_description` behavior remains available for task 05’s existing token-endpoint regression.
- [ ] No certificate file is created; test output is the completion evidence.
