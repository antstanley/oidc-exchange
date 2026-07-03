# Done Certificate — Task 04: per-invocation synchronous flush

**Task:** [04-per_invocation_flush.md](04-per_invocation_flush.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

> This certificate is a verification protocol for Task 04. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 04) ≡ every obligation O1…O5 below holds, each backed by the evidence the obligation
names (a file location, an execution trace, or a test result) — not by assertion.

## Premises

- **P1 — Goal.** The task wraps the Lambda run path with a per-invocation synchronous flush hook
  that force-flushes telemetry (and buffered audit, when an adapter buffers) after each
  invocation's response future resolves and before the response is returned — the single seam the
  telemetry-exporters change later fills.
- **P2 — Obligations.** The task is done iff O1…O5 all hold. One Oi per definition-of-done item,
  in DoD order; O5 is the `Reviewable:` item.
- **P3 — Invariants.** Must not break Task 03's Lambda run path (`main.rs` Lambda branch) or the
  hyper path; the flush wrapper factors out of `main.rs` without changing the router, middleware,
  or the hyper branch behaviour.

## Obligations

- **O1 — Flush runs synchronously per invocation, before the response returns.**
  - *Claim:* a per-invocation flush hook runs after each Lambda invocation's response future
    resolves and before the response is returned, on the wrapper factored out of `main.rs`.
  - *Evidence to collect:* read the wrapper unit (e.g. `crates/server/src/lambda.rs`) and
    `flush_telemetry` beside `init_telemetry` in `crates/server/src/telemetry.rs:16`; trace one
    invocation through the wrapper and confirm the flush call sits after the inner service future
    resolves and before the response is yielded to `lambda_http`.
  - *Checks:* resolve the flush call — confirm it invokes the server crate's `flush_telemetry`
    (or the injected flush), not a same-named local or a no-op that is never called.
  - *Status:* ☑ SATISFIED — `crates/server/src/lambda.rs`: `FlushOnResponse::call` (lines
    57–67) does `let fut = self.inner.call(req); … let result = fut.await; flush(); result`,
    so the flush fires *after* the inner router future resolves and *before* the response is
    returned. `run_lambda` (lines 74–79) wraps `app` in `FlushOnResponse::new(app, flush)` and
    serves it via `lambda_http::run`. `main.rs:48` calls
    `lambda::run_lambda(app, Arc::new(telemetry::flush_telemetry))`. Resolution: the injected
    `flush` is `crate::telemetry::flush_telemetry` (telemetry.rs:86), invoked via `flush()` in
    `call` — not a same-named local, not a call-less no-op. It is a documented no-op *body*
    today but is genuinely invoked on every call.

- **O2 — Negative space: flush fires on the error path too.**
  - *Claim:* the flush fires on a non-200/error invocation as well as a success one.
  - *Evidence to collect:* run the flush integration test — confirm it drives two sequential
    invocations (one 200, one error status) with an injected flush spy and asserts the flush count
    increments once per invocation on both paths.
  - *Status:* ☑ SATISFIED — `lambda::tests::flush_fires_once_per_invocation_on_success_and_error_paths`
    drives `/ok` (resolves 200) then `/error` (resolves 500) through one wrapped service with an
    injected atomic counter, asserting the counter is 1 after the success invocation and 2 after
    the error invocation. Ran targeted: `cargo nextest run -p oidc-exchange --lib lambda::` → 2
    passed (this test + `flush_fires_even_when_no_route_matches`, a 404 no-matched-handler case
    that also fires the flush). The error/non-200 path does not skip the flush.

- **O3 — Seam documented, defensively asserted.**
  - *Claim:* the flush seam is documented as the single point telemetry and buffered audit flush
    through, and the wrapper/flush function carries ≥2 meaningful assertions.
  - *Evidence to collect:* read the wrapper and `flush_telemetry` — confirm a `// why` comment
    naming the seam (telemetry now, buffered audit when an adapter buffers) and ≥2 assertions
    (e.g. flush-count precondition/postcondition per invocation).
  - *Checks:* confirm the audit-flush path is a documented no-op today by resolving against
    `AuditLog` (`crates/core/src/ports/audit.rs`) — no `flush` method — and `SqsAuditLog::emit`
    (`crates/adapters/src/sqs_audit/mod.rs:33`), which sends per event.
  - *Status:* ☑ SATISFIED — the seam is documented in two places: the module doc of
    `crates/server/src/lambda.rs` (lines 1–16) names it as the single per-invocation flush point
    for telemetry now and buffered audit when an adapter buffers, citing the exporters change;
    `flush_telemetry`'s doc (`crates/server/src/telemetry.rs:70–85`) restates it is the single
    `force_flush` seam. ≥2 meaningful assertions: the flush-count precondition/postcondition
    assertions (counter == 1 after invocation 1, == 2 after invocation 2), each with an
    explanatory message, exactly the example the obligation names. Audit no-op check resolved:
    `AuditLog` (`crates/core/src/ports/audit.rs:7–9`) declares only `emit`, no `flush`;
    `SqsAuditLog::emit` (`crates/adapters/src/sqs_audit/mod.rs:33`) serializes and `send_message`s
    per event — nothing buffered — so the audit-flush leg is a documented no-op today.

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass, format and clippy clean, any bound is a named constant.
  - *Evidence to collect:* run the repo's Rust gates from
    [.specs/development-guidelines.md](../../../development-guidelines.md) §Definition of done —
    `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`,
    `cargo nextest run --workspace` — expect all clean.
  - *Status:* ☑ SATISFIED — `cargo fmt --all --check` → exit 0; `cargo clippy --workspace
    -- -D warnings` → exit 0 (Finished clean, no warnings); `cargo nextest run --workspace` →
    387 tests run, 387 passed, 27 skipped. No new numeric bound introduced in the flush code
    (no magic limits — the flush is a hook, not a bounded loop/buffer).

- **O5 — Reviewable: flush fires after each of two sequential invocations (Reviewable).**
  - *Claim:* a reviewer runs the per-invocation-flush test and sees the flush hook fire after each
    of two sequential invocations (success and error).
  - *Evidence to collect:* run the flush integration test and observe the flush-spy count reaching
    2 across the two invocations, one per invocation.
  - *Status:* ☑ SATISFIED — exercised, not assumed: `cargo nextest run -p oidc-exchange --lib
    lambda::` ran `flush_fires_once_per_invocation_on_success_and_error_paths` → PASS, with the
    injected spy counter observed at 1 after the first (200) invocation and 2 after the second
    (error) invocation — the flush hook fires once after each of the two sequential invocations.

## Regression check

- Task 03's Lambda branch still serves the router: trace the wrapped Lambda path and confirm a
  `/keys` event still returns 200 + JWKS with the flush wrapper in place (the flush does not alter
  the response) : ☑ PRESERVED — `FlushOnResponse` (`lambda.rs:44–68`) delegates `poll_ready`
  and `call` straight to the inner router and returns the inner `Result` unchanged; `flush()`
  runs only as a side effect after `fut.await`, so the response body/status is never altered.
  `run_lambda` serves the same shared `app` router Task 03 built. The router's `/keys` JWKS path
  is covered by `oidc-exchange-ffi::integration test_jwks_endpoint` (plus `test_openid_discovery`,
  `test_health_endpoint`) — all PASS in the workspace run — and the wrapper is transparent to it.
  Router, middleware, and the hyper branch are untouched.

## Residue

Notes for the validator: the tracer-provider `force_flush` behind the seam is inert until the
OTLP/X-Ray exporters land via
[changes/2026-06-24-complete_telemetry_exporters.md](../../../changes/2026-06-24-complete_telemetry_exporters.md);
this task certifies the seam and its per-invocation invocation, not the flushing of a live OTLP
pipeline. Confirm the flush hook is invoked (via the spy) even though its telemetry effect is
currently a no-op.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O5 all SATISFIED with evidence — `FlushOnResponse` fires the injected
`telemetry::flush_telemetry` hook after each invocation's response future resolves and before it
is returned, the flush-spy test proves it fires once per invocation on both the 200 and error
paths, the seam is documented (audit leg a confirmed no-op: no `AuditLog::flush`, `SqsAuditLog`
sends per event), fmt/clippy/nextest (387 passed) are clean, and the `/keys` router path is
PRESERVED because the wrapper returns the inner response unchanged.
