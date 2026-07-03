# Done Certificate — Task 01: per-request tracing span

**Task:** [01-per_request_span.md](01-per_request_span.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

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
  - *Status:* ☑ SATISFIED — `request_id.rs:46-51` builds `info_span!("request", request_id = %request_id,
    method = %method, path = %path)` and line 52 runs `next.run(request).instrument(span).await`.
    Resolution check: `.instrument` is `tracing::Instrument::instrument` (trait imported at line 4
    `use tracing::Instrument;`) applied to the `next.run(request)` future — it enters the span across
    the await, not a bare `Span::current().record`. No shadowing (the only `instrument` in scope is
    that trait method). Tests `handler_log_carries_generated_request_id` and
    `handler_log_carries_reused_request_id` both PASS: the event emitted inside the handler is captured
    with `request_id` equal to the echoed header value, for the generated and reused cases respectively.

- **O2 — Negative-space: malformed/absent `X-Request-Id` still yields a valid generated id.**
  - *Claim:* a request without a usable `X-Request-Id` gets a valid generated UUID on the span
    and the response header.
  - *Evidence to collect:* run `request_id.rs` tests `generates_request_id_when_absent` and
    `preserves_existing_request_id` — expect PASS. Confirm the generated id is a valid UUID on
    the response header.
  - *Status:* ☑ SATISFIED — `generates_request_id_when_absent` and `preserves_existing_request_id`
    both PASS. Two extra negative-space tests also PASS: `generates_valid_request_id_when_header_is_malformed`
    (non-UTF-8 header bytes) and `generates_valid_request_id_when_header_is_empty` (present-but-blank
    header). Each asserts the response `x-request-id` is a valid 36-char UUID
    (`uuid::Uuid::parse_str(id).is_ok()`). The `.filter(|s| !s.is_empty())` at `request_id.rs:27`
    makes a blank header fall through to a generated UUID.

- **O3 — `request_id_layer` carries at least two meaningful assertions.**
  - *Claim:* the function has two or more non-trivial assertions (not `assert!(true)`).
  - *Evidence to collect:* read `request_id_layer`; count the `assert!`/`debug_assert!` calls
    and confirm each guards a real property (e.g. non-empty id, header set before return).
  - *Status:* ☑ SATISFIED — two non-trivial `assert!`s. Precondition at `request_id.rs:34-37`
    (`!request_id.is_empty()`) guards a usable correlation id; postcondition at `:65-68`
    (`response.headers().get(REQUEST_ID_HEADER).is_some()`) guards the echoed header being present
    before the response leaves the middleware. Neither is `assert!(true)`; each guards a real property.

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass and lint/format are clean.
  - *Evidence to collect:* run `cargo fmt --all --check`, `cargo clippy --workspace -- -D
    warnings`, `cargo nextest run --workspace` — expect all clean.
  - *Status:* ☑ SATISFIED — `cargo fmt --all --check` exit 0 (clean); `cargo clippy --workspace
    --all-targets -- -D warnings` exit 0 with 0 warnings/errors; `cargo nextest run --workspace`
    → 347 passed, 0 failed, 27 skipped. Named-constant limits honoured: the magic `"x-request-id"`
    literal is now the `const REQUEST_ID_HEADER` (`request_id.rs:7`), used at both the read and echo sites.

- **O5 — Reviewable: handler log captured with the correct `request_id`, header still echoed.**
  - *Claim:* a reviewer sees a handler-emitted log event captured with the correct `request_id`
    and confirms the response echoes `X-Request-Id`.
  - *Evidence to collect:* run the request-id middleware tests and inspect the captured event's
    `request_id` field; confirm the response header assertion passes.
  - *Status:* ☑ SATISFIED — exercised, not assumed. `handler_log_carries_generated_request_id` and
    `handler_log_carries_reused_request_id` install a `RequestIdCaptureLayer` tracing subscriber,
    drive a request through `request_id_layer` + a `tracing::info!`-emitting handler, and assert the
    captured event's `request_id` equals the response's echoed `x-request-id` header (a concrete
    reused value `reused-id-456` in the reused case). Both PASS; the response-header echo is asserted
    in each.

## Regression check

- `crates/server/src/bootstrap.rs:134` layers `request_id_layer` onto the router; trace that the
  layered middleware still compiles and the existing E2E in `crates/server/tests/e2e.rs` sees
  `X-Request-Id` on responses : ☑ PRESERVED — the layering now lives at `bootstrap.rs:317`
  (`app.layer(axum::middleware::from_fn(request_id_layer))`), still the innermost/last layer; the
  whole workspace compiles clean under clippy and all 347 tests pass, so the middleware still wires in.
  Note: `crates/server/tests/e2e.rs` does not itself assert on `X-Request-Id` (no `request_id`
  reference in `crates/server/tests/`), so that specific certificate wording is slightly stale; the
  header echo is instead proven by the `request_id.rs` module tests, which all pass. The
  reuse-or-generate logic and the echo insertion are unchanged in substance — only wrapped in a span —
  so no downstream caller regresses.

## Residue

- If the tower OTEL request-span layer from the telemetry change spec has already landed, confirm
  task 01 records `request_id` on the existing span rather than nesting a second — outside this
  DoD but noted by the plan's no-nesting decision.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O5 all SATISFIED with evidence — the handler-emitted log carries `request_id` for both the
generated and reused cases (proven by two `RequestIdCaptureLayer` tests), the negative-space and blank/
malformed-header cases yield a valid generated UUID, `request_id_layer` carries two meaningful pre/post
assertions, and fmt + clippy(-D warnings) + the full 347-test workspace all pass; the bootstrap layering
is PRESERVED.

## Residue (validator note)

No tower OTEL request-span layer is layered in `bootstrap.rs` yet (only `request_id_layer`,
`audit_context_layer`, `CatchPanicLayer`), so `request_id_layer` is currently the innermost layer and
owns the per-request span via the `is_disabled()` `if` branch. The `else` branch that folds `request_id`
into a pre-existing outer span (the no-nesting decision) is present and correct for when that layer
lands, but is not exercised by any current test — outside this task's DoD, noted per the Residue item.
