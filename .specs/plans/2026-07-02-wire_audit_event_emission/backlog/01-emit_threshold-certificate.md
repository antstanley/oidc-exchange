# Done Certificate — Task 01: emit threshold filter

**Task:** [01-emit_threshold.md](01-emit_threshold.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified

> This certificate is a verification protocol for Task 01. A validating agent discharges it: for each
> obligation, collect the named evidence, run the named checks, set the Status, then derive the
> Conclusion by the rubric below. Do not mark an obligation SATISFIED without its evidence; do not
> record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 01) ≡ every obligation O1…O5 below holds, each backed by the evidence it names (a file
location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** `emit_audit` drops events strictly less severe than `[audit] emit_threshold`
  before any adapter dispatch; `emit_threshold` is a config key defaulting to `info`.
- **P2 — Obligations.** Done iff O1…O5 all hold, in DoD order; O5 is the Reviewable item.
- **P3 — Invariants.** Must not break the existing fallback-and-blocking-threshold behaviour of
  `emit_audit` (`crates/core/src/service/mod.rs:102-129`) or the tests in `crates/core/tests/audit.rs`.

## Obligations

- **O1 — Pre-dispatch suppression.**
  - *Claim:* `emit_audit` returns `Ok(())` without calling `self.audit.emit` when the event
    severity is strictly less severe than `emit_threshold`; otherwise it dispatches and keeps the
    existing fallback/blocking behaviour.
  - *Evidence to collect:* read `mod.rs:102` onward; confirm a threshold check precedes the
    `self.audit.emit(&event).await` call and returns `Ok(())` on `event.severity as u8 > threshold as u8`.
  - *Checks:* resolve `parse_severity` at the new call site to `crates/core/src/service/mod.rs:153`,
    not a shadow; confirm the severity comparison direction matches "strictly less severe" (higher
    numeric syslog value = less severe).
  - *Status:* ☐ unverified

- **O2 — Named config field with `info` default.**
  - *Claim:* `AuditConfig` has `emit_threshold: String` defaulting to `"info"`; the comparison
    reuses `parse_severity` with no new numeric severity literal.
  - *Evidence to collect:* read `crates/core/src/config.rs:81` (struct) and `:87` (`Default`);
    confirm the field and `"info"` default. Grep the new `emit_audit` code for numeric severity
    literals — expect none beyond the existing `as u8` comparisons.
  - *Status:* ☐ unverified

- **O3 — Negative-space test: debug suppressed under `info`.**
  - *Claim:* a `Debug`-severity event under the default `info` threshold never reaches the adapter.
  - *Evidence to collect:* run the new suppression test (in `crates/core/tests/audit.rs`) — expect
    PASS, asserting `MockAuditLog::events()` is empty after `emit_audit` of a debug event.
  - *Checks:* resolve the mock's recorded-events accessor to `MockAuditLog` in
    `crates/test-utils`, confirming it records only dispatched events.
  - *Status:* ☐ unverified

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean, limits named.
  - *Evidence to collect:* run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`,
    `cargo nextest run --workspace` — expect all clean.
  - *Status:* ☐ unverified

- **O5 — Reviewable: suppression test passes, blocking tests unchanged.**
  - *Claim:* the new suppression test passes and the existing blocking-threshold tests still pass.
  - *Evidence to collect:* run `cargo nextest run -p oidc-exchange-core audit` — expect PASS,
    including `blocking_audit_failure_warning_event_warning_threshold` and the new suppression test.
  - *Status:* ☐ unverified

## Regression check

- `crates/core/tests/audit.rs` callers of `emit_audit` (info/warning/error events) exercise the
  dispatch path after the new pre-filter → expect unchanged Ok/Err outcomes : ☐ (PRESERVED / REGRESSION)

## Residue

- Whether `config/default.toml` should pin `emit_threshold` explicitly or rely on the struct default
  is a presentation choice, not an obligation. Note only.

## Conclusion

VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
