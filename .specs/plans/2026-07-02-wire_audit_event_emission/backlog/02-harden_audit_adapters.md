# Task 02 — harden audit adapters

**Plan:** [plan.md](../plan.md) · **Certificate:** [02-harden_audit_adapters-certificate.md](02-harden_audit_adapters-certificate.md)

**Implements:** [07-telemetry-and-audit.md](../../../service/specs/07-telemetry-and-audit.md) §Audit (the adapter write-failure sentence and the FIFO `message_group_id`/deduplication detail)
**Depends on:** —
**Produces:** the stdout adapter returns `Error::AuditError` on a write failure instead of panicking; the sqs adapter detects FIFO with `ends_with(".fifo")` and sets `message_group_id` to the event id.
**Pointers:** `crates/adapters/src/stdout_audit/mod.rs:47-51` (the `println!`/`eprintln!` writes); `crates/adapters/src/sqs_audit/mod.rs:60` (`contains(".fifo")`), `mod.rs:69-70` (`message_deduplication_id` / `message_group_id`), `mod.rs:110-117` (`test_fifo_detection`)

## Steps

- [ ] In `stdout_audit/mod.rs`, replace `println!`/`eprintln!` with `writeln!` to a locked `io::stdout()`/`io::stderr()` handle, mapping any `io::Error` to `Error::AuditError { detail }`.
- [ ] In `sqs_audit/mod.rs:60`, change the FIFO guard from `contains(".fifo")` to `ends_with(".fifo")`.
- [ ] In `sqs_audit/mod.rs:70`, set `message_group_id` to `&event.id` (each event its own group); leave `message_deduplication_id` as the event's ULID (`mod.rs:69`) and drop the now-unused serialized `event_type` grouping.
- [ ] Update `test_fifo_detection` to assert `ends_with(".fifo")` semantics (a non-suffix `.fifo` substring is rejected); add a stdout negative-space test that a write to a broken handle surfaces `Err(AuditError)` rather than panicking.

## Definition of done

- [ ] The stdout adapter writes via locked handles and returns `Err(Error::AuditError { .. })` on a write error (e.g. EPIPE) instead of panicking, so the failure reaches `emit_audit`'s fallback-and-threshold path.
- [ ] The sqs adapter's FIFO detection is suffix-based (`ends_with(".fifo")`) and its `message_group_id` is the event id, with the ULID deduplication id retained.
- [ ] Negative-space test: a broken/failing stdout write yields `Err(AuditError)` (asserted, not a panic); a URL containing `.fifo` mid-string is treated as a standard (non-FIFO) queue.
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: run `cargo nextest run -p oidc-exchange-adapters` and observe the updated `test_fifo_detection` and the new stdout write-failure test pass.
