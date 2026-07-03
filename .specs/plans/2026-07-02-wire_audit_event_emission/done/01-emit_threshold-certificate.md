# Done Certificate — Task 01: emit threshold filter

**Task:** [01-emit_threshold.md](01-emit_threshold.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

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
  - *Status:* SATISFIED — `crates/core/src/service/mod.rs:106-110`: the threshold check
    (`parse_severity(&self.config.audit.emit_threshold).unwrap_or(AuditSeverity::Info)` then
    `if event.severity as u8 > emit_threshold as u8 { return Ok(()); }`) precedes
    `self.audit.emit(&event).await` at `:112`. `parse_severity` resolves to the same-module
    `pub fn parse_severity` at `mod.rs:162` — the only definition in `crates/core/src`, no shadow.
    `AuditSeverity` is syslog-ordered (`domain/audit.rs:26-35`, Emergency=0 … Debug=7), so
    `event as u8 > threshold as u8` is exactly "strictly less severe". The fallback/blocking
    branch (`mod.rs:112-137`) is untouched by the diff.

- **O2 — Named config field with `info` default.**
  - *Claim:* `AuditConfig` has `emit_threshold: String` defaulting to `"info"`; the comparison
    reuses `parse_severity` with no new numeric severity literal.
  - *Evidence to collect:* read `crates/core/src/config.rs:81` (struct) and `:87` (`Default`);
    confirm the field and `"info"` default. Grep the new `emit_audit` code for numeric severity
    literals — expect none beyond the existing `as u8` comparisons.
  - *Status:* SATISFIED — `crates/core/src/config.rs:185` declares `pub emit_threshold: String`
    on `AuditConfig`; `Default` at `:194` sets `"info"`. The new `emit_audit` filter contains no
    numeric severity literal — only `parse_severity(...)`, `AuditSeverity::Info` as fallback, and
    an `as u8` cast comparison matching the existing style. `config/default.toml` also pins
    `emit_threshold = "info"`.

- **O3 — Negative-space test: debug suppressed under `info`.**
  - *Claim:* a `Debug`-severity event under the default `info` threshold never reaches the adapter.
  - *Evidence to collect:* run the new suppression test (in `crates/core/tests/audit.rs`) — expect
    PASS, asserting `MockAuditLog::events()` is empty after `emit_audit` of a debug event.
  - *Checks:* resolve the mock's recorded-events accessor to `MockAuditLog` in
    `crates/test-utils`, confirming it records only dispatched events.
  - *Status:* SATISFIED — ran `cargo nextest run -p oidc-exchange-core audit`:
    `audit_debug_event_under_default_emit_threshold_is_suppressed` PASS. The test asserts
    `result.is_ok()` and `audit_clone.events().await` is empty under `AppConfig::default()`
    (which it first asserts carries `emit_threshold == "info"`). `MockAuditLog::events()`
    resolves to `crates/test-utils/src/lib.rs:333`; events are pushed only inside
    `AuditLog::emit` (`lib.rs:350-358`), so the mock records only dispatched events.

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean, limits named.
  - *Evidence to collect:* run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`,
    `cargo nextest run --workspace` — expect all clean.
  - *Status:* SATISFIED — `cargo fmt --check` clean (exit 0); `cargo clippy --workspace -- -D warnings`
    clean (exit 0); `cargo nextest run --workspace` → 310 tests run, 310 passed, 27 skipped.
    No new numeric limits introduced (the only constants touched are named severity strings).

- **O5 — Reviewable: suppression test passes, blocking tests unchanged.**
  - *Claim:* the new suppression test passes and the existing blocking-threshold tests still pass.
  - *Evidence to collect:* run `cargo nextest run -p oidc-exchange-core audit` — expect PASS,
    including `blocking_audit_failure_warning_event_warning_threshold` and the new suppression test.
  - *Status:* SATISFIED — `cargo nextest run -p oidc-exchange-core audit` → 7 tests run, 7 passed:
    the three new tests (`audit_debug_event_under_default_emit_threshold_is_suppressed`,
    `audit_info_event_at_default_emit_threshold_is_dispatched`,
    `audit_debug_event_reaches_adapter_when_threshold_lowered_to_debug`) plus the four pre-existing
    tests including `blocking_audit_failure_warning_event_warning_threshold` — all PASS, none modified.

## Regression check

- `crates/core/tests/audit.rs` callers of `emit_audit` (info/warning/error events) exercise the
  dispatch path after the new pre-filter → expect unchanged Ok/Err outcomes : PRESERVED —
  the four pre-existing `crates/core/tests/audit.rs` tests pass unchanged. Trace: their events are
  Info(6)/Warning(4)/Error(3), all ≤ the default `info` (6) emit threshold, so none is dropped by
  the pre-filter; each reaches the untouched fallback/blocking branch and yields the same Ok/Err
  outcome as before.

## Residue

- Whether `config/default.toml` should pin `emit_threshold` explicitly or rely on the struct default
  is a presentation choice, not an obligation. Note only.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O5 all SATISFIED with direct evidence — the pre-dispatch filter at
`service/mod.rs:106-110` reuses `parse_severity` (mod.rs:162, no shadow) with the correct
strictly-less-severe comparison, `emit_threshold: String` defaults to `"info"`
(config.rs:185/:194 and config/default.toml), the suppression/dispatch/lowered-threshold tests
and all 310 workspace tests pass with fmt+clippy clean, and the existing blocking-threshold
tests are PRESERVED. Residue note: `config/default.toml` pins `emit_threshold = "info"` explicitly.
