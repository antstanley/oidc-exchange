# Task 04 — per-invocation synchronous flush

**Plan:** [plan.md](../plan.md) · **Certificate:** [04-per_invocation_flush-certificate.md](04-per_invocation_flush-certificate.md)

**Implements:** [service/specs/04-http-api.md](../../../service/specs/04-http-api.md) §Bootstrap (step 6 — "In Lambda mode, telemetry and blocking audit writes flush synchronously before each invocation's response is returned, since the execution environment may freeze immediately after the response")
**Depends on:** 03
**Produces:** in Lambda mode, telemetry (and buffered audit writes, when an adapter buffers) force-flush synchronously after each invocation's response future resolves and before the response is returned — a single per-invocation flush seam that the telemetry-exporters change later fills
**Pointers:** `crates/server/src/main.rs:29-33` (the Lambda run branch from Task 03); `crates/server/src/telemetry.rs:16` (`init_telemetry` — where a `flush_telemetry()` companion belongs); `crates/adapters/src/sqs_audit/mod.rs:33` (`emit` sends per event today — no buffer, so the audit flush is a documented no-op until an adapter buffers)

## Steps

- [ ] Add a `flush_telemetry()` (or equivalently named) function beside `init_telemetry` in `crates/server/src/telemetry.rs` that force-flushes the installed telemetry pipeline; under the current stdout-JSON subscriber it is a safe no-op, and it is the single point where the tracer-provider `force_flush` lands when the exporters change ships.
- [ ] Wrap the Lambda run path so the flush runs synchronously after each invocation's response future resolves and before the response is returned — factor the wrapper into a testable unit in the server crate (e.g. `crates/server/src/lambda.rs`), not inline in `main.rs`, so it can be exercised without a live runtime.
- [ ] Document (comment + task note) that buffered audit adapters flush through this same hook; today no adapter buffers (`AuditLog` has no `flush`, `SqsAuditLog::emit` awaits per event), so the audit path is a no-op seam.
- [ ] Add an integration test that injects a flush spy/counter into the wrapper and asserts the flush hook fires exactly once per invocation across two sequential invocations — one that returns 200 and one that returns an error status — proving the flush runs on both the success and error paths.

## Definition of done

- [ ] A per-invocation flush hook runs synchronously after each Lambda invocation's response resolves, before the response is returned, on the wrapper factored out of `main.rs`.
- [ ] Negative-space test: the flush fires on the error/non-200 invocation as well as the success one (paired success/error invocations, flush count asserted per invocation).
- [ ] The flush seam is documented as the single point telemetry and buffered audit flush through, with ≥2 meaningful assertions in the wrapper/flush function.
- [ ] Meets the repo definition of done (tests, `cargo fmt`, `cargo clippy --workspace -- -D warnings`, named-constant limits — see plan.md baseline).
- [ ] Reviewable: run the per-invocation-flush test and observe the flush hook firing after each of two sequential invocations (success and error).

## Open questions

- The full tracer-provider `force_flush` behind this seam depends on
  [changes/2026-06-24-complete_telemetry_exporters.md](../../../changes/2026-06-24-complete_telemetry_exporters.md)
  landing; until then the flush hook is a no-op under the stdout-JSON pipeline. This task builds
  and tests the seam; the exporters change populates it.
