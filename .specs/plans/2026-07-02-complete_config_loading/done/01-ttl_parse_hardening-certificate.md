# Done Certificate — Task 01: TTL parse hardening

**Task:** [01-ttl_parse_hardening.md](01-ttl_parse_hardening.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

> This certificate is a verification protocol for Task 01. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 01) ≡ every obligation O1…O5 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** `parse_duration_secs` cannot panic on any input and cannot overflow silently — malformed or overflowing durations return `ConfigError`, well-formed ones return the exact second count.
- **P2 — Obligations.** The task is done iff O1…O5 all hold. One Oi per definition-of-done item, in DoD order; O5 is the `Reviewable:` item.
- **P3 — Invariants.** Existing callers of `parse_duration_secs` (the per-request TTL sites in `crates/core/src/service/`) still receive the correct second count for well-formed input.

## Obligations

- **O1 — Correct count for valid units; `ConfigError` (never panic) for bad input.**
  - *Claim:* `parse_duration_secs` returns the right seconds for `"45s"`/`"15m"`/`"1h"`/`"30d"` and returns `Err(ConfigError)` — not a panic — for multi-byte, empty, unknown-suffix, and overflowing input.
  - *Evidence collected:* `crates/core/src/service/mod.rs:180-231` — the suffix is split via `split_at(s.len() - suffix_char.len_utf8())` where `suffix_char = s.chars().next_back()` (char-boundary, not `s.len()-1`), and `m`/`h`/`d` use `value.checked_mul(SECONDS_PER_{MINUTE,HOUR,DAY})`; empty, unknown-suffix, non-numeric, and overflow each return `Error::ConfigError { detail }`. `cargo nextest run -p oidc-exchange-core parse_duration` → 9 passed.
  - *Checks:* the multi-byte case `"15€"` is handled by the char-boundary split (test `parse_duration_secs_multi_byte_final_char_does_not_panic` returns `Err`, no panic).
  - *Status:* ☑ SATISFIED

- **O2 — Negative-space tests for the previously-panicking paths.**
  - *Claim:* dedicated tests cover a multi-byte final char and an overflowing value.
  - *Evidence collected:* `service::parse_duration_secs_tests::parse_duration_secs_multi_byte_final_char_does_not_panic` (multi-byte suffix → `Err`) and `..parse_duration_secs_overflowing_day_count_is_rejected` (u64-scale day count → `Err` via `checked_mul`) both PASS.
  - *Status:* ☑ SATISFIED

- **O3 — Named multiplier constants and meaningful assertions.**
  - *Claim:* the seconds-per-minute/hour/day multipliers are named constants; the function/tests carry at least two meaningful assertions.
  - *Evidence collected:* `SECONDS_PER_MINUTE`/`SECONDS_PER_HOUR`/`SECONDS_PER_DAY` consts (mod.rs:169-172), each referenced by name in the match arms; no bare `3600`/`86400` literal remains. The function carries two postcondition `assert_eq!`s (partition + single-char suffix); tests assert both values and `Err` variants.
  - *Status:* ☑ SATISFIED

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests, lint, and format are clean per the repo guidelines.
  - *Evidence collected:* `cargo fmt --check` exit 0; `cargo clippy --workspace --all-targets -- -D warnings` finished clean; `cargo nextest run --workspace` → 155 passed, 2 skipped (8 new tests over the 147 baseline).
  - *Status:* ☑ SATISFIED

- **O5 — Reviewable: multi-byte and overflow cases return `Err`, not a panic.**
  - *Claim:* a reviewer runs the targeted tests and sees the previously-crashing inputs handled as errors.
  - *Evidence collected:* `cargo nextest run -p oidc-exchange-core parse_duration` → 9 tests passed including the multi-byte and overflow cases; no panic/abort in output.
  - *Status:* ☑ SATISFIED

## Regression check

- The per-request TTL call sites in `crates/core/src/service/` call `parse_duration_secs` with the configured TTLs → the pre-existing `service::exchange::tests::parse_duration_secs_works` test still PASSES, so well-formed values yield the same second count : ☑ PRESERVED

## Residue

- None noted at authoring. Reuse of the parsed value at request time (so request paths cannot re-fail) is exercised via Task 02's `validate()` wiring, not here.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: All five obligations SATISFIED against `crates/core/src/service/mod.rs` — char-boundary split removes the multi-byte panic, `checked_mul` turns overflow into `ConfigError`, unit multipliers are named constants, 8 non-vacuous tests cover every path (valid s/m/h/d, multi-byte, overflow, empty, unknown suffix), and fmt/clippy/nextest are clean. Existing caller behaviour preserved.
