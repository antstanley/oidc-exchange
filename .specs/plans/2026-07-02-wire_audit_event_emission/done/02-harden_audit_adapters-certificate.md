# Done Certificate — Task 02: harden audit adapters

**Task:** [02-harden_audit_adapters.md](02-harden_audit_adapters.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

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
  - *Status:* ☑ SATISFIED — `stdout_audit/mod.rs:48-52` now calls `write_line(io::stderr().lock(), &json)?` /
    `write_line(io::stdout().lock(), &json)?`; `io` resolves to `std::io` via the `use std::io::{self, Write}`
    import at `mod.rs:11`, so `.lock()` yields `StderrLock`/`StdoutLock` (both `impl Write`) — not the
    `println!`/`eprintln!` macros. `write_line` (`mod.rs:62-66`, same-module private fn, no shadow) uses
    `writeln!` with `.map_err(|e| Error::AuditError { detail })`, propagated by `?` at both call sites;
    no `.unwrap()`/panic on the write path.

- **O2 — sqs suffix detection and per-event group.**
  - *Claim:* the FIFO guard is `ends_with(".fifo")` and `message_group_id` is `&event.id`, with the
    ULID `message_deduplication_id` retained.
  - *Evidence to collect:* read `crates/adapters/src/sqs_audit/mod.rs:60` and `:69-70`; confirm
    `ends_with(".fifo")`, `message_group_id(&event.id)`, and `message_deduplication_id(&event.id)`.
  - *Status:* ☑ SATISFIED — `sqs_audit/mod.rs:60` reads `if self.queue_url.ends_with(".fifo")`;
    `mod.rs:62-63` (lines shifted from the authored :69-70 by the deleted `event_type_str` block) set
    `.message_deduplication_id(&event.id)` and `.message_group_id(&event.id)`; the serialized
    `event_type` grouping is gone from the diff.

- **O3 — Negative-space tests.**
  - *Claim:* a broken stdout write yields `Err(AuditError)` (not a panic); a `.fifo` substring
    mid-URL is treated as a standard queue.
  - *Evidence to collect:* run `cargo nextest run -p oidc-exchange-adapters` — expect the new
    stdout write-failure test and the updated `test_fifo_detection` (asserting a mid-string `.fifo`
    is non-FIFO) PASS.
  - *Status:* ☑ SATISFIED — ran `cargo nextest run -p oidc-exchange-adapters`: 124 passed, 0 failed.
    `stdout_audit::tests::write_line_surfaces_broken_handle_as_audit_error` PASS (a `BrokenWriter`
    returning EPIPE yields `Err(Error::AuditError { detail })` asserted via `expect_err`, no panic);
    `sqs_audit::tests::test_fifo_detection` PASS (asserts `...audit-events.fifo-archive`, a mid-string
    `.fifo`, is NOT FIFO). Per the Residue note, the broken write is exercised via `write_line` with a
    fake `Write` target — the allowed injection mechanism.

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean, limits named.
  - *Evidence to collect:* run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`,
    `cargo nextest run --workspace` — expect all clean.
  - *Status:* ☑ SATISFIED — `cargo fmt --check` clean (exit 0); `cargo clippy --workspace -- -D warnings`
    clean; `cargo nextest run --workspace`: 311 passed, 0 failed. The diff introduces no new numeric
    limits, so no named-constant work applies.

- **O5 — Reviewable: adapter tests pass.**
  - *Claim:* the updated `test_fifo_detection` and the new stdout write-failure test pass.
  - *Evidence to collect:* run `cargo nextest run -p oidc-exchange-adapters` — expect PASS.
  - *Status:* ☑ SATISFIED — exercised: `cargo nextest run -p oidc-exchange-adapters` → 124 passed,
    including `test_fifo_detection` and `write_line_surfaces_broken_handle_as_audit_error`.

## Regression check

- `emit_audit` (`crates/core/src/service/mod.rs:102`) calls `self.audit.emit`; with the stdout
  adapter now returning `Err` on write failure, trace that the error flows into `emit_audit`'s
  fallback path (not a panic) → expect the fallback tracing + threshold decision : ☑ PRESERVED —
  traced `emit_audit` (`crates/core/src/service/mod.rs:102`): `self.audit.emit(&event).await` at :112;
  an `Err(Error::AuditError)` from the stdout adapter enters the `Err(e)` arm (:114), is serialized and
  emitted via `tracing::error!/info!(audit_fallback = true, ...)` (:116-123), then the blocking-threshold
  decision (:126-135) returns `Err(e)` or `Ok(())` — no panic path. P3 invariants hold: `emit_stdout_succeeds`,
  `emit_stderr_succeeds`, `emit_auto_routes_by_severity`, and `test_event_serializes_to_json` all PASS,
  and the sqs body/attribute build (`sqs_audit/mod.rs:53-58`) is untouched by the diff.

## Residue

- The stdout write-failure test may need a fake `Write` target rather than the real stdout handle;
  the injection mechanism is an implementation detail, not an obligation.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O5 all SATISFIED with collected evidence (locked-handle `write_line` mapping io errors to
`Error::AuditError`, suffix-based FIFO detection with per-event `message_group_id`/ULID dedup id, both
negative-space tests passing, fmt/clippy/workspace suite clean at 311 tests), and the `emit_audit`
fallback-and-threshold caller is PRESERVED.
