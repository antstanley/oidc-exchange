# Task 01 — per-request tracing span

**Plan:** [plan.md](../plan.md) · **Certificate:** [01-per_request_span-certificate.md](01-per_request_span-certificate.md)

**Implements:** [04-http-api.md](../../../service/specs/04-http-api.md) §Middleware stack (entry 1, Request ID) and [07-telemetry-and-audit.md](../../../service/specs/07-telemetry-and-audit.md) §request-id correlation (its existing claim becomes accurate once the span exists)
**Depends on:** —
**Produces:** every log emitted while handling a request carries the `request_id` field, so request-id correlation is real rather than a no-op.
**Pointers:** `crates/server/src/middleware/request_id.rs:18` (the no-op `tracing::Span::current().record("request_id", ...)`), `:10-25` (`request_id_layer`).

## Steps

- [x] Replace the no-op `record` call: build `tracing::info_span!("request", request_id = %request_id, method = %request.method(), path = %request.uri().path())` and run `next.run(request)` instrumented with it via `tracing::Instrument::instrument`.
- [x] Keep the reuse-or-generate `X-Request-Id` logic and the response-header echo exactly as they are; only the span is new.
- [x] Guard against a duplicate span if an outer request span already exists (per the plan's no-nesting decision) — record `request_id` on the current span when one is already open, else open the `info_span`.
- [x] Add two meaningful assertions to `request_id_layer` (e.g. assert the generated/parsed id is a valid non-empty UUID or reused value; assert the response carries the echoed header before returning).
- [x] Extend the module tests: assert that a log event emitted inside the handler is captured with the `request_id` field set (use a `tracing` test subscriber), for both the generated and the reused-header cases.

## Definition of done

- [x] A log emitted during request handling carries the request's `request_id` field (generated case and reused-`X-Request-Id` case both proven by test).
- [x] Negative-space: a request with a malformed/absent `X-Request-Id` still yields a valid generated id on the span and the response header (existing `generates_request_id_when_absent` test still passes).
- [x] `request_id_layer` carries at least two meaningful assertions.
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: a reviewer runs the request-id middleware tests and sees a handler-emitted log event captured with the correct `request_id`, and confirms the response still echoes `X-Request-Id`.
