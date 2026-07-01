# Done Certificate — Task 01: per-request tracing span

**Task:** [01-per_request_span.md](01-per_request_span.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified

> Verification protocol for Task 01. A validating agent discharges it: for each obligation,
> collect the named evidence, run the named checks, set the Status, then derive the Conclusion
> by the rubric. Do not mark an obligation SATISFIED without its evidence; do not record DONE
> with any non-SATISFIED obligation.

## Definition

DONE(Task 01) ≡ every obligation O1…O5 below holds, each backed by the evidence it names — not
by assertion.

## Premises

- **P1 — Goal.** The task makes request-id correlation real: every log emitted while handling a
  request carries the request's `request_id` field, via a per-request span in the request-id
  middleware.
- **P2 — Obligations.** Done iff O1…O5 all hold, in DoD order; O5 is the Reviewable item.
- **P3 — Invariants.** Must not break the existing `X-Request-Id` reuse-or-generate logic or the
  response-header echo in `request_id_layer` (`crates/server/src/middleware/request_id.rs`).

## Obligations

- **O1 — A handler-emitted log carries `request_id` (generated and reused cases).**
  - *Claim:* a `tracing` event emitted inside the handler is captured with `request_id` set,
    for both a generated id and a reused `X-Request-Id`.
  - *Evidence to collect:* read `request_id.rs` around the replaced `record` call; confirm an
    `info_span!("request", request_id = …)` is built and `next.run(request)` is instrumented
    with it. Run the module tests (`cargo nextest run -p oidc-exchange-server request_id`) — expect
    the new span-capture tests PASS for both cases.
  - *Checks:* resolve the instrumenting call — confirm it is `tracing::Instrument::instrument`
    on the `next.run` future (or an equivalent `.in_scope`/entered span covering the await), not
    a bare `Span::current().record` that leaves no active span. Flag if the span is created but
    never entered/instrumented.
  - *Status:* ☐ unverified

- **O2 — Negative-space: malformed/absent `X-Request-Id` still yields a valid generated id.**
  - *Claim:* a request without a usable `X-Request-Id` gets a valid generated UUID on the span
    and the response header.
  - *Evidence to collect:* run `request_id.rs` tests `generates_request_id_when_absent` and
    `preserves_existing_request_id` — expect PASS. Confirm the generated id is a valid UUID on
    the response header.
  - *Status:* ☐ unverified

- **O3 — `request_id_layer` carries at least two meaningful assertions.**
  - *Claim:* the function has two or more non-trivial assertions (not `assert!(true)`).
  - *Evidence to collect:* read `request_id_layer`; count the `assert!`/`debug_assert!` calls
    and confirm each guards a real property (e.g. non-empty id, header set before return).
  - *Status:* ☐ unverified

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass and lint/format are clean.
  - *Evidence to collect:* run `cargo fmt --all --check`, `cargo clippy --workspace -- -D
    warnings`, `cargo nextest run --workspace` — expect all clean.
  - *Status:* ☐ unverified

- **O5 — Reviewable: handler log captured with the correct `request_id`, header still echoed.**
  - *Claim:* a reviewer sees a handler-emitted log event captured with the correct `request_id`
    and confirms the response echoes `X-Request-Id`.
  - *Evidence to collect:* run the request-id middleware tests and inspect the captured event's
    `request_id` field; confirm the response header assertion passes.
  - *Status:* ☐ unverified

## Regression check

- `crates/server/src/bootstrap.rs:134` layers `request_id_layer` onto the router; trace that the
  layered middleware still compiles and the existing E2E in `crates/server/tests/e2e.rs` sees
  `X-Request-Id` on responses : ☐ (PRESERVED / REGRESSION)

## Residue

- If the tower OTEL request-span layer from the telemetry change spec has already landed, confirm
  task 01 records `request_id` on the existing span rather than nesting a second — outside this
  DoD but noted by the plan's no-nesting decision.

## Conclusion

VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
