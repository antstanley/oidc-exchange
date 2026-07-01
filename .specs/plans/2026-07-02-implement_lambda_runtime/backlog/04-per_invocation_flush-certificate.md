# Done Certificate — Task 04: per-invocation synchronous flush

**Task:** [04-per_invocation_flush.md](04-per_invocation_flush.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified   <!-- validator sets: Validated YYYY-MM-DD -->

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
  - *Status:* ☐ unverified

- **O2 — Negative space: flush fires on the error path too.**
  - *Claim:* the flush fires on a non-200/error invocation as well as a success one.
  - *Evidence to collect:* run the flush integration test — confirm it drives two sequential
    invocations (one 200, one error status) with an injected flush spy and asserts the flush count
    increments once per invocation on both paths.
  - *Status:* ☐ unverified

- **O3 — Seam documented, defensively asserted.**
  - *Claim:* the flush seam is documented as the single point telemetry and buffered audit flush
    through, and the wrapper/flush function carries ≥2 meaningful assertions.
  - *Evidence to collect:* read the wrapper and `flush_telemetry` — confirm a `// why` comment
    naming the seam (telemetry now, buffered audit when an adapter buffers) and ≥2 assertions
    (e.g. flush-count precondition/postcondition per invocation).
  - *Checks:* confirm the audit-flush path is a documented no-op today by resolving against
    `AuditLog` (`crates/core/src/ports/audit.rs`) — no `flush` method — and `SqsAuditLog::emit`
    (`crates/adapters/src/sqs_audit/mod.rs:33`), which sends per event.
  - *Status:* ☐ unverified

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass, format and clippy clean, any bound is a named constant.
  - *Evidence to collect:* run the repo's Rust gates from
    [.specs/development-guidelines.md](../../../development-guidelines.md) §Definition of done —
    `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`,
    `cargo nextest run --workspace` — expect all clean.
  - *Status:* ☐ unverified

- **O5 — Reviewable: flush fires after each of two sequential invocations (Reviewable).**
  - *Claim:* a reviewer runs the per-invocation-flush test and sees the flush hook fire after each
    of two sequential invocations (success and error).
  - *Evidence to collect:* run the flush integration test and observe the flush-spy count reaching
    2 across the two invocations, one per invocation.
  - *Status:* ☐ unverified

## Regression check

- Task 03's Lambda branch still serves the router: trace the wrapped Lambda path and confirm a
  `/keys` event still returns 200 + JWKS with the flush wrapper in place (the flush does not alter
  the response) : ☐ (PRESERVED / REGRESSION)

## Residue

Notes for the validator: the tracer-provider `force_flush` behind the seam is inert until the
OTLP/X-Ray exporters land via
[changes/2026-06-24-complete_telemetry_exporters.md](../../../changes/2026-06-24-complete_telemetry_exporters.md);
this task certifies the seam and its per-invocation invocation, not the flushing of a live OTLP
pipeline. Confirm the flush hook is invoked (via the spy) even though its telemetry effect is
currently a no-op.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
