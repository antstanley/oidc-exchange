# Task 02 — Bound and validate request IDs

**Status:** Backlog · **Plan:** [plan.md](../plan.md) · **Certificate:** forbidden and intentionally omitted

**Implements:** [source spec §04-http-api — Middleware stack](../../../changes/2026-08-05-eliminate_secret_leakage_in_logs_and_spans.md#specsservicespecs04-http-apimd--middleware-stack-item-1-modify); [§Implementation notes step 2](../../../changes/2026-08-05-eliminate_secret_leakage_in_logs_and_spans.md#implementation-notes); [`04-http-api.md` Middleware stack](../../../service/specs/04-http-api.md)
**Depends on:** —
**Produces:** a named 128-byte request-ID limit and predicate that preserves acceptable correlation IDs and silently replaces all other inbound values with UUIDv4 IDs.
**Pointers:** `crates/server/src/middleware/request_id.rs`; its module tests and existing `preserves_existing_request_id` test.

## Steps

- [ ] Add `MAX_REQUEST_ID_LEN: usize = 128` and `is_acceptable_request_id(&str) -> bool` with explicit non-empty, length, and ASCII `[A-Za-z0-9_-]` checks.
- [ ] Use the predicate in the inbound-header path; do not log rejected values and do not turn malformed correlation metadata into a request failure.
- [ ] Keep request span recording and response-header echo behavior unchanged for accepted and generated IDs.
- [ ] Add boundary tests for exactly 128 bytes, 129 bytes, a 64 KiB input, a legal-ASCII but wrongly shaped value, and invalid characters; keep accepted-id reuse and absent/invalid generation tests.

## Task-specific definition of done

- [ ] Only plausible IDs are reused; every rejected input yields a valid generated UUIDv4 in both span/response behavior.
- [ ] The limit is named and tests cover below/at/above boundary behavior plus charset negative space.
- [ ] Rejection remains silent: capture tests show the inbound malformed value is not emitted.
- [ ] No certificate file is created; test output is the completion evidence.
