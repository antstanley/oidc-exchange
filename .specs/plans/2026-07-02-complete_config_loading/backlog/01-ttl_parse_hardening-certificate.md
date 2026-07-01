# Done Certificate — Task 01: TTL parse hardening

**Task:** [01-ttl_parse_hardening.md](01-ttl_parse_hardening.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified   <!-- validator sets: Validated YYYY-MM-DD -->

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
  - *Evidence to collect:* read `crates/core/src/service/mod.rs` around the rewritten function; confirm the last character is split on a char boundary (not `split_at(s.len()-1)`) and each multiplication uses `checked_mul`. Run `cargo nextest run -p oidc-exchange-core parse_duration` — expect all cases PASS with the malformed cases returning `Err`.
  - *Checks:* confirm the char-boundary split handles a multi-byte final char (e.g. `"15€"`) by returning `Err`, not indexing inside the char.
  - *Status:* ☐ unverified

- **O2 — Negative-space tests for the previously-panicking paths.**
  - *Claim:* dedicated tests cover a multi-byte final char and an overflowing value.
  - *Evidence to collect:* locate the new `#[test]` cases in `crates/core/src/service/mod.rs`; confirm one feeds a multi-byte-suffix string and one feeds a value large enough to overflow the day multiplier, each asserting `Err`.
  - *Status:* ☐ unverified

- **O3 — Named multiplier constants and meaningful assertions.**
  - *Claim:* the seconds-per-minute/hour/day multipliers are named constants; the function/tests carry at least two meaningful assertions.
  - *Evidence to collect:* grep the module for numeric literals `60`/`3600`/`86400`; confirm each is a named `const` referenced by name. Confirm the tests assert both a value and an error variant.
  - *Status:* ☐ unverified

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests, lint, and format are clean per the repo guidelines.
  - *Evidence to collect:* run `cargo fmt --all -- --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` (per `.specs/development-guidelines.md` §Definition of done) — expect all clean.
  - *Status:* ☐ unverified

- **O5 — Reviewable: multi-byte and overflow cases return `Err`, not a panic.**
  - *Claim:* a reviewer runs the targeted tests and sees the previously-crashing inputs handled as errors.
  - *Evidence to collect:* run `cargo nextest run -p oidc-exchange-core parse_duration` and observe the multi-byte and overflow cases pass by returning `Err` (no panic/abort in output).
  - *Status:* ☐ unverified

## Regression check

- The per-request TTL call sites in `crates/core/src/service/` call `parse_duration_secs` with the configured TTLs → expect the same second count as before for well-formed values : ☐ (PRESERVED / REGRESSION)

## Residue

- None noted at authoring. Reuse of the parsed value at request time (so request paths cannot re-fail) is exercised via Task 02's `validate()` wiring, not here.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
