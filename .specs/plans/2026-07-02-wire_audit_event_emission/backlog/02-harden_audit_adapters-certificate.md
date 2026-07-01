# Done Certificate — Task 02: harden audit adapters

**Task:** [02-harden_audit_adapters.md](02-harden_audit_adapters.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified

> This certificate is a verification protocol for Task 02. A validating agent discharges it: collect
> each obligation's evidence, run its checks, set the Status, then derive the Conclusion by the rubric.
> Do not mark an obligation SATISFIED without its evidence; do not record DONE with any non-SATISFIED
> obligation.

## Definition

DONE(Task 02) ≡ every obligation O1…O5 below holds, each backed by the evidence it names — not by
assertion.

## Premises

- **P1 — Goal.** The stdout adapter returns `AuditError` on write failure instead of panicking; the
  sqs adapter detects FIFO with `ends_with(".fifo")` and sets `message_group_id` to the event id.
- **P2 — Obligations.** Done iff O1…O5 all hold, in DoD order; O5 is the Reviewable item.
- **P3 — Invariants.** Must not break the successful-emit paths in `stdout_audit/mod.rs` (the
  `emit_stdout_succeeds`/`emit_auto_routes_by_severity` tests) or the sqs message-body/attribute
  build (`sqs_audit/mod.rs:53-58`, `test_event_serializes_to_json`).

## Obligations

- **O1 — stdout returns Err on write failure.**
  - *Claim:* the stdout adapter writes via locked `io::stdout()`/`io::stderr()` handles and returns
    `Err(Error::AuditError { .. })` on an io write error rather than panicking.
  - *Evidence to collect:* read `crates/adapters/src/stdout_audit/mod.rs:47-51` (rewritten);
    confirm `writeln!` to a locked handle with the `io::Error` mapped to `Error::AuditError`.
  - *Checks:* resolve `writeln!`'s target to a `std::io::Stdout`/`Stderr` lock, not the `println!`
    macro; confirm the `?`/`map_err` propagates rather than `.unwrap()`/panic.
  - *Status:* ☐ unverified

- **O2 — sqs suffix detection and per-event group.**
  - *Claim:* the FIFO guard is `ends_with(".fifo")` and `message_group_id` is `&event.id`, with the
    ULID `message_deduplication_id` retained.
  - *Evidence to collect:* read `crates/adapters/src/sqs_audit/mod.rs:60` and `:69-70`; confirm
    `ends_with(".fifo")`, `message_group_id(&event.id)`, and `message_deduplication_id(&event.id)`.
  - *Status:* ☐ unverified

- **O3 — Negative-space tests.**
  - *Claim:* a broken stdout write yields `Err(AuditError)` (not a panic); a `.fifo` substring
    mid-URL is treated as a standard queue.
  - *Evidence to collect:* run `cargo nextest run -p oidc-exchange-adapters` — expect the new
    stdout write-failure test and the updated `test_fifo_detection` (asserting a mid-string `.fifo`
    is non-FIFO) PASS.
  - *Status:* ☐ unverified

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean, limits named.
  - *Evidence to collect:* run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`,
    `cargo nextest run --workspace` — expect all clean.
  - *Status:* ☐ unverified

- **O5 — Reviewable: adapter tests pass.**
  - *Claim:* the updated `test_fifo_detection` and the new stdout write-failure test pass.
  - *Evidence to collect:* run `cargo nextest run -p oidc-exchange-adapters` — expect PASS.
  - *Status:* ☐ unverified

## Regression check

- `emit_audit` (`crates/core/src/service/mod.rs:102`) calls `self.audit.emit`; with the stdout
  adapter now returning `Err` on write failure, trace that the error flows into `emit_audit`'s
  fallback path (not a panic) → expect the fallback tracing + threshold decision : ☐ (PRESERVED / REGRESSION)

## Residue

- The stdout write-failure test may need a fake `Write` target rather than the real stdout handle;
  the injection mechanism is an implementation detail, not an obligation.

## Conclusion

VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
