# Task 02 — Bound and validate request IDs

**Status:** Done · **Plan:** [plan.md](../plan.md) · **Certificate:** forbidden and intentionally omitted

**Implements:** [source spec §04-http-api — Middleware stack](../../../changes/2026-08-05-eliminate_secret_leakage_in_logs_and_spans.md#specsservicespecs04-http-apimd--middleware-stack-item-1-modify); [§Implementation notes step 2](../../../changes/2026-08-05-eliminate_secret_leakage_in_logs_and_spans.md#implementation-notes); [`04-http-api.md` Middleware stack](../../../service/specs/04-http-api.md)
**Depends on:** —
**Produces:** a named 128-byte request-ID limit and predicate that preserves acceptable correlation IDs and silently replaces all other inbound values with UUIDv4 IDs.
**Pointers:** `crates/server/src/middleware/request_id.rs`; its module tests and existing `preserves_existing_request_id` test.

## Steps

- [x] Add `MAX_REQUEST_ID_LEN: usize = 128` and `is_acceptable_request_id(&str) -> bool` with explicit non-empty, length, and ASCII `[A-Za-z0-9_-]` checks.
- [x] Use the predicate in the inbound-header path; do not log rejected values and do not turn malformed correlation metadata into a request failure.
- [x] Keep request span recording and response-header echo behavior unchanged for accepted and generated IDs.
- [x] Add boundary tests for exactly 128 bytes, 129 bytes, a 64 KiB input, a legal-ASCII but wrongly shaped value, and invalid characters; keep accepted-id reuse and absent/invalid generation tests.

## Task-specific definition of done

- [x] Only plausible IDs are reused; every rejected input yields a valid generated UUIDv4 in both span/response behavior.
- [x] The limit is named and tests cover below/at/above boundary behavior plus charset negative space.
- [x] Rejection remains silent: capture tests show the inbound malformed value is not emitted.
- [x] No certificate file is created; test output is the completion evidence.

**Evidence:** server crate — all 13 `request_id`-related nextest tests pass (predicate boundary/charset sweep; at-limit reuse vs one-byte-over and 64 KiB rejection to fresh UUIDv4; wrongly-shaped ASCII replacement; `rejected_request_ids_are_never_logged` capture test with positive controls proving non-vacuousness). Pre-existing `preserves_existing_request_id`, empty/malformed-UTF-8 generation, and timeout-correlation tests still green.
