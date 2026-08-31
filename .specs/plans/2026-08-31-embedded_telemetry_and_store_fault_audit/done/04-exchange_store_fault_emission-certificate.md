# Done Certificate — Task 04: Exchange flow emits the operational store-fault event

**Task:** [04-exchange_store_fault_emission.md](04-exchange_store_fault_emission.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-08-31

> Verification protocol for Task 04. A validating agent discharges it: collect each obligation's
> evidence, run its checks, set the Status, then derive the Conclusion by the rubric. Do not mark
> an obligation SATISFIED without its evidence; do not record DONE with any non-SATISFIED
> obligation.

## Definition

DONE(Task 04) ≡ every obligation O1…O6 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** The exchange flow's `StoreError` early return records one best-effort
  operational audit event (`store_error`, severity `error`, outcome `failure`/`store_error`,
  `actor: None`, provider/`client_addr`/`user_agent` from the request,
  `detail.store_detail` carrying the store's diagnostic) and then propagates the original
  `StoreError` — the emission result is discarded, and no terminal `SecurityEvent` is added.
- **P2 — Obligations.** Done iff O1…O6 all hold, one per definition-of-done item in DoD order;
  O6 is the Reviewable item.
- **P3 — Invariants.** Must not break: every non-`StoreError` exchange outcome's terminal
  emission (`exchange_mandatory_outcomes.rs`); the error the caller receives on a store fault
  (`Error::StoreError`, mapped to 5xx at the HTTP boundary — never an `AuditError` or
  `SecurityAuditDurability`); the arm's classification (no `SecurityEvent`, no client
  attribution); and the updated relookup test's original intent (no user/session created, one
  lookup). Task 03 (the `StoreError` variants) is a precondition — confirm it is in `done/` or
  its variants present before discharging.

## Obligations

- **O1 — The happy-path fault test proves one `store_error` event with the full shape and no terminal `SecurityEvent`.**
  - *Claim:* an exchange against a failing store returns `Error::StoreError` while the recording
    sink holds exactly one event with `event_type: store_error`, `severity: error`,
    `outcome: failure`/`store_error`, the provider named, `actor` absent, `ip_address`/
    `user_agent` from the request, and non-empty `detail.store_detail` — and no other event.
  - *Evidence to collect:* read the modified arm in `crates/core/src/service/exchange.rs`
    (pre-change lines 158-162): confirm it assembles via `create_audit_event`
    (`crates/core/src/service/mod.rs:368-391` pre-change) with exactly the fields above, inserts
    the `StoreError`'s `detail` string under the key `store_detail`, emits via `emit_audit`, and
    returns the original error. Run the new flow test beside `crates/core/tests/exchange.rs`
    (using `FailingCreateUserRepository` or a failing session store, and `MockAuditLog`) — expect
    PASS with assertions on every listed field and on `events.len() == 1`.
  - *Checks:* resolve the emission call — confirm it is `self.emit_audit`
    (`mod.rs:255-274` pre-change, the threshold-filtered best-effort path), not
    `emit_security_event` / `emit_mandatory_audit_event`. Confirm the detail key is
    `store_detail`, not `detail` (the change spec's namespacing decision). Confirm both `Other`
    routes reach the arm: direct store failures and `AssertionBindError::Store`
    (`exchange.rs:567-579` pre-change) — by construction both produce
    `ExchangeFlowError::Other(Error::StoreError { .. })`; cite the match arm that catches them.
  - *Status:* SATISFIED — arm at `exchange.rs:165-188` (workspace) assembles via
    `create_audit_event` (resolved to the imported free fn, `mod.rs:368-391`) with exactly the
    listed fields, inserts the diagnostic under `store_detail` (:177-180), emits via
    `self.emit_audit` (resolved to `mod.rs:255`, the threshold-filtered best-effort path — not
    `emit_security_event`/`emit_mandatory_audit_event`), and returns the original error (:187).
    Both `Other` routes reach it: direct store failures via `From<Error>` (`exchange.rs:471,480`
    and `?`) and `AssertionBindError::Store → Other(err)` (`exchange.rs:600`) — caught by the
    `Err(ExchangeFlowError::Other(Error::StoreError { detail }))` arm at :165. New test
    `exchange_store_fault_emits_one_operational_store_error_event` PASS, asserting `len() == 1`,
    every listed field, and the serialized JSON shape (no `detail.detail`).

- **O2 — Best-effort semantics pinned: a raised threshold suppresses the event without changing the response.**
  - *Claim:* with `emit_threshold` above `Error` (e.g. `critical`), no event is emitted and the
    returned error is unchanged.
  - *Evidence to collect:* run the named threshold test — expect PASS with the sink empty and the
    result still `Error::StoreError`. Trace the config path: `emit_threshold = critical (2)` <
    `Error (3)` numerically → `emit_audit`'s severity gate (`event.severity as u8 >
    emit_threshold as u8`) returns `Ok` without emitting.
  - *Status:* SATISFIED — `exchange_store_fault_event_respects_raised_emit_threshold` PASS: sink
    empty, result still `Error::StoreError`. Trace confirmed: `Critical = 2`, `Error = 3`
    (`audit.rs:43-51`); gate at `mod.rs:257` sees `3 > 2` → `Ok(())` before the sink is touched.

- **O3 — The discard is pinned twice: failing sink (observe) and failing sink under `durability = "enforce"` both still return `StoreError`.**
  - *Claim:* an audit-sink failure never displaces the store fault: with `MockAuditLog` in
    `fail_mode` the flow returns `Error::StoreError` (never `AuditError`), and with
    `audit.durability = "enforce"` plus a failing sink it still returns `Error::StoreError`
    (never `SecurityAuditDurability`) — the mandatory-channel durability contract does not govern
    this event.
  - *Evidence to collect:* run both named tests — expect PASS. Read the arm: confirm the
    emission result is discarded (e.g. `let _ =` with a why-comment) rather than matched into the
    return value. Trace the failing-sink path: `emit_audit` → sink `Err` → `log_audit_fallback`
    (`mod.rs:348-355` pre-change) logs the serialized event → `emit_audit` may return `Err`
    (severity `Error` ≤ default `blocking_threshold = warning`? `warning (4)` ≥ `error (3)`, so
    the blocking branch returns `Err`) → the arm discards it → caller gets `StoreError`. The
    discard is load-bearing precisely because `emit_audit` can return `Err` here.
  - *Checks:* resolve `log_audit_fallback` — confirm the discard's safety argument (the event was
    already logged) holds by reading `emit_audit`'s `Err` branch, which calls it before
    returning.
  - *Status:* SATISFIED — both tests PASS
    (`exchange_store_fault_with_failing_audit_sink_still_returns_store_error`,
    `exchange_store_fault_under_enforce_durability_still_returns_store_error`). The arm discards
    with `let _ = self.emit_audit(event).await;` plus a why-comment (`exchange.rs:181-186`).
    Trace: sink `Err` → `log_audit_fallback` (`mod.rs:264` → `:348-355`, logs the serialized
    event at error level) → `Error (3) <= blocking Warning (4)` → `emit_audit` returns `Err`
    (`mod.rs:266-267`) — the discard is load-bearing exactly as authored. `audit.durability` is
    consulted only in `emit_mandatory_audit_event` (`mod.rs:332`), never on this path.

- **O4 — `exchange_mandatory_outcomes.rs` passes unchanged; the updated relookup test preserves its original assertions.**
  - *Claim:* client-fault outcomes gain no store-fault event, and
    `exchange_non_conflict_create_error_propagates_without_relookup` now expects exactly one
    `store_error` event while keeping its no-user/no-session/one-lookup assertions.
  - *Evidence to collect:* diff `crates/core/tests/exchange_mandatory_outcomes.rs` — expect no
    edits; run it — expect PASS. Read the updated relookup test
    (`crates/core/tests/exchange.rs`, pre-change lines 1239-1304): the former
    `events.is_empty()` assertion (pre-change `exchange.rs:1282-1290`) now asserts exactly one `store_error` event and no
    terminal `SecurityEvent`-classed event, while `get_all_users`/`get_all_sessions` emptiness
    and the single-lookup count remain. Run the full `exchange.rs` binary — expect PASS.
  - *Status:* SATISFIED — `jj diff -r @ --git crates/core/tests/exchange_mandatory_outcomes.rs`
    is empty (byte-unchanged); its suite plus `assertion` PASS (20/20). The updated relookup test
    now asserts `events.len() == 1` and `events[0].event_type == AuditEventType::StoreError`
    (tests/exchange.rs:1284-1295) while keeping `StoreError` propagation (:1274-1277),
    `get_all_users`/`get_all_sessions` emptiness (:1299-1300), and the single-lookup count
    (:1304-1308). Full exchange binary PASS (30/30).

- **O5 — Meets the repo definition of done.**
  - *Claim:* format, lint, and the workspace test suite pass, and the touched arm carries
    meaningful assertions or a recorded reason.
  - *Evidence to collect:* per `.specs/development-guidelines.md` §Definition of done, run
    `cargo fmt --check --all`, `cargo clippy --workspace -- -D warnings`, and
    `cargo nextest run --workspace` — expect all clean/green. Confirm `AppService::exchange`
    stays within the review-gate bounds or the change description records why not.
  - *Status:* SATISFIED — `cargo fmt --check --all` exit 0; `cargo clippy --workspace --
    -D warnings` clean; `cargo nextest run --workspace` 933 passed / 0 failed / 78 skipped
    (baseline 929 + the 4 new tests). The new tests carry dense, meaningful assertions.
    `AppService::exchange` grows to ~124 lines — over the 70-line review gate, but that gate is
    reviewer-enforced by explicit decision (development-guidelines.md records existing functions
    up to 134 lines and declines the lint), and the function already exceeded 70 before this
    change; noted, not a DoD failure.

- **O6 — Reviewable: run the core exchange suite and inspect the recorded `store_error` event's JSON shape (Reviewable).**
  - *Claim:* a reviewer can run the exchange tests and read the new test's assertions as the
    event's concrete shape — the `store_detail` string being the same diagnostic class the 500
    mapping logs.
  - *Evidence to collect:* run `cargo nextest run -p oidc-exchange-core -E 'binary(exchange)'` —
    expect PASS; open the new test and confirm its assertions spell out the full event shape
    (not a bare `len() == 1`).
  - *Status:* SATISFIED — ran `cargo nextest run -p oidc-exchange-core -E 'binary(exchange)'` →
    30/30 PASS. The new test's assertions spell out the full shape field-by-field and pin the
    serialized JSON (`event_type: "store_error"`, `severity: "error"`, `outcome.status:
    "failure"`, `outcome.reason: "store_error"`, `detail.store_detail` equal to the store's
    diagnostic, and no nested `detail.detail`) — the same diagnostic class the 500 mapping logs
    (`crates/server/src/error.rs` store-error test).

## Regression check

- The HTTP boundary maps the propagated error: `crates/server/src/error.rs` 500 mapping receives
  `Error::StoreError` from `/token` → expect `500 {"error":"server_error"}` with the internal
  `Display` logged, exactly as before (the arm still returns the original error)
  : PRESERVED — `crates/server/src/error.rs:203-211` maps `StoreError` → 500 `server_error`;
  the arm rebuilds `Error::StoreError { detail }` with the identical single field (error.rs:47);
  the server crate's own store-error mapping test passes in the workspace run.
- Successful exchanges: `crates/core/tests/exchange.rs` happy-path tests → expect the terminal
  `AuthenticationSucceeded` emission unchanged and no `store_error` event recorded
  : PRESERVED — the success path never enters the `StoreError` arm (early return is new code on
  a previously silent path); all exchange happy-path tests pass (30/30).
- Binding rejections (`ExchangeFlowError::Binding`) → expect the detail-enriched
  `ValidationFailed` terminal event unchanged (`crates/core/tests/assertion.rs`)
  : PRESERVED — the `Binding` arm is textually untouched and arm order is unchanged; the
  `assertion` test binary passes.

## Residue

- Refresh and revoke store faults stay un-audited by the change spec's explicit Decision; their
  extension is its Open question — not a gap in this task.
- The canonical prose for `03-service-flows.md` and `07-telemetry-and-audit.md` travels with the
  change spec's Merge plan, not this task.
- Severity ordering is inverted (RFC 5424: lower is more severe); validators comparing thresholds
  should trace the `as u8` comparisons rather than assuming higher-is-worse.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric:
NOT_DONE — any load-bearing obligation UNSATISFIED, or a REGRESSION found.
PARTIAL — all obligations SATISFIED except one or more UNVERIFIED, and no regression.
DONE — every obligation SATISFIED, regression PRESERVED, evidence sufficient for each. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O6 all SATISFIED with collected evidence (arm read and calls resolved, all four new
tests plus the updated relookup test PASS, `exchange_mandatory_outcomes.rs` byte-unchanged and
green, fmt/clippy clean, workspace 933/933 with 78 skipped, the Reviewable command run and the
JSON-shape assertions inspected), and all three named regression callers PRESERVED.
