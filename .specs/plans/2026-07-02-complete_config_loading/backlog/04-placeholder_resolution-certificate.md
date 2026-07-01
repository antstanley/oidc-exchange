# Done Certificate — Task 04: Placeholder resolution

**Task:** [04-placeholder_resolution.md](04-placeholder_resolution.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified   <!-- validator sets: Validated YYYY-MM-DD -->

> This certificate is a verification protocol for Task 04. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 04) ≡ every obligation O1…O5 below holds, each backed by the evidence the obligation
names — not by assertion.

## Premises

- **P1 — Goal.** A post-merge pass replaces every `${VAR}` string value with the environment value, aborts with `ConfigError` on an unset variable (fail closed), and rewrites `$${` to a literal `${` without treating it as a placeholder opener.
- **P2 — Obligations.** Done iff O1…O5 all hold; O5 is the `Reviewable:` item.
- **P3 — Invariants.** The Task 03 merge/override behaviour in `load_config` is preserved; the resolver runs after it and does not alter non-placeholder values.

## Obligations

- **O1 — Placeholder resolves when set; unset fails closed; literal never survives.**
  - *Claim:* `shared_secret = "${INTERNAL_API_SECRET}"` yields the env value when the var is set; with the var unset `load_config` returns `Err` and the literal `${INTERNAL_API_SECRET}` never reaches the config.
  - *Evidence to collect:* read the resolver in `crates/server/src/bootstrap.rs`; confirm an unset variable produces `ConfigError` whose `detail` names the variable and that no branch leaves the literal in place. Run the set-var and unset-var tests — expect the value and an `Err` respectively.
  - *Checks:* trace the unset-variable path and confirm it returns `Err` before deserialization, so no `AppConfig` with a literal placeholder is ever produced.
  - *Status:* ☐ unverified

- **O2 — `$${` escapes to a literal `${`.**
  - *Claim:* `$${INTERNAL_API_SECRET}` resolves to the literal string `${INTERNAL_API_SECRET}` and is never looked up in the environment.
  - *Evidence to collect:* run the escape test — expect the literal output; confirm the escape is handled such that `$${` is never treated as an opener even when the named var is unset (it must not error).
  - *Status:* ☐ unverified

- **O3 — Negative-space tests and named bounds.**
  - *Claim:* tests cover the unset-variable fail-closed path and the escape path; any introduced bound (e.g. a scan limit) is a named constant.
  - *Evidence to collect:* locate the `#[test]` cases; confirm one asserts `Err` on unset and one asserts the literal on `$${`. Grep for any numeric bound introduced by the resolver and confirm it is a named `const`.
  - *Status:* ☐ unverified

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests, lint, format clean.
  - *Evidence to collect:* run `cargo fmt --all -- --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` (per `.specs/development-guidelines.md` §Definition of done) — expect all clean.
  - *Status:* ☐ unverified

- **O5 — Reviewable: set resolves, unset aborts naming the var, escape becomes literal.**
  - *Claim:* loading a config with `${SET_VAR}`, `${UNSET_VAR}`, and `$${LITERAL}` resolves the first, aborts on the second (error names `UNSET_VAR`), and yields `${LITERAL}` for the third.
  - *Evidence to collect:* run the combined test (or a manual `load_config` under a temp config) and observe the three outcomes, including the error message naming `UNSET_VAR`.
  - *Status:* ☐ unverified

## Regression check

- `load_config`'s callers (server bootstrap; and after Task 05, the FFI `parse_config` path) still load a config with no placeholders unchanged → expect identical values to pre-resolution : ☐ (PRESERVED / REGRESSION)

## Residue

- None noted at authoring. Validation that consumes the resolved secret is wired in Task 05.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
