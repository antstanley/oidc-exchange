# Done Certificate — Task 04: Placeholder resolution

**Task:** [04-placeholder_resolution.md](04-placeholder_resolution.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

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
  - *Status:* ☑ SATISFIED — resolver read at `crates/server/src/bootstrap.rs` (`resolve_placeholders_in_str`): an unset variable hits `std::env::var(name).map_err(...)` producing `Error::ConfigError { detail: "config placeholder '${NAME}' references unset environment variable 'NAME'" }` — the detail names the variable. `Error` resolves to `oidc_exchange_core::error::Error` (bootstrap.rs:9 import; variant at crates/core/src/error.rs:49), not the `config` crate's error — no shadowing. Trace: `load_config` calls `resolve_placeholders(&mut merged.cache)?` *before* `merged.try_deserialize()`, so the `Err` propagates before any `AppConfig` exists. Tests `set_var_placeholder_resolves_to_its_environment_value` and `unset_var_placeholder_fails_closed_and_produces_no_config` both PASS (cargo nextest).

- **O2 — `$${` escapes to a literal `${`.**
  - *Claim:* `$${INTERNAL_API_SECRET}` resolves to the literal string `${INTERNAL_API_SECRET}` and is never looked up in the environment.
  - *Evidence to collect:* run the escape test — expect the literal output; confirm the escape is handled such that `$${` is never treated as an opener even when the named var is unset (it must not error).
  - *Status:* ☑ SATISFIED — test `escaped_placeholder_yields_literal_dollar_brace_without_env_lookup` PASS: `$${LITERAL_NOT_A_VAR}` with the variable deliberately unset (`remove_var` in the test) loads successfully and yields the literal `${LITERAL_NOT_A_VAR}` — proving no env lookup occurred (an unset lookup would fail closed). Code: the escape branch in `resolve_placeholders_in_str` matches `$${` first, emits `${`, and consumes all three bytes so the `{` can never re-open a placeholder.

- **O3 — Negative-space tests and named bounds.**
  - *Claim:* tests cover the unset-variable fail-closed path and the escape path; any introduced bound (e.g. a scan limit) is a named constant.
  - *Evidence to collect:* locate the `#[test]` cases; confirm one asserts `Err` on unset and one asserts the literal on `$${`. Grep for any numeric bound introduced by the resolver and confirm it is a named `const`.
  - *Status:* ☑ SATISFIED — `unset_var_placeholder_fails_closed_and_produces_no_config` asserts `expect_err` and that the message contains `INTERNAL_API_SECRET`; `escaped_placeholder_yields_literal_dollar_brace_without_env_lookup` asserts the literal. The one introduced bound is the named constant `PLACEHOLDER_NAME_LEN_MAX: usize = 256` (bootstrap.rs:47) used in `scan_placeholder_name`; no bare numeric bounds in the resolver. Additional coverage: unchanged plain value and nested map-valued section tests also PASS.

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests, lint, format clean.
  - *Evidence to collect:* run `cargo fmt --all -- --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` (per `.specs/development-guidelines.md` §Definition of done) — expect all clean.
  - *Status:* ☑ SATISFIED — `cargo fmt --all -- --check` clean; `cargo clippy --workspace -- -D warnings` clean; `cargo nextest run --workspace` → 178 passed, 0 failed, 2 skipped (both pre-existing `#[ignore]` tests requiring live KMS/DynamoDB — unrelated to this task).

- **O5 — Reviewable: set resolves, unset aborts naming the var, escape becomes literal.**
  - *Claim:* loading a config with `${SET_VAR}`, `${UNSET_VAR}`, and `$${LITERAL}` resolves the first, aborts on the second (error names `UNSET_VAR`), and yields `${LITERAL}` for the third.
  - *Evidence to collect:* run the combined test (or a manual `load_config` under a temp config) and observe the three outcomes, including the error message naming `UNSET_VAR`.
  - *Status:* ☑ SATISFIED — combined test `set_unset_and_escaped_placeholders_together_match_reviewable_scenario` PASS: a config with `${SET_VAR}` + `${UNSET_VAR}` fails the whole load with an error whose message contains `UNSET_VAR`; after swapping the unset placeholder for `$${LITERAL}`, the reload resolves `${SET_VAR}` to `resolved-value` and yields the literal `${LITERAL}`.

## Regression check

- `load_config`'s callers (server bootstrap; and after Task 05, the FFI `parse_config` path) still load a config with no placeholders unchanged → expect identical values to pre-resolution : ☑ PRESERVED — `value_without_a_placeholder_is_unchanged` PASS, and all pre-existing `load_config` merge/override tests (Task 03 surface) pass in the full workspace run; the only insertion into `load_config` is `resolve_placeholders(&mut merged.cache)?` between build and deserialize, which is an identity transform on placeholder-free strings. The FFI `parse_config` path does not exist yet (Task 05).

## Residue

- None noted at authoring. Validation that consumes the resolved secret is wired in Task 05.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O5 all SATISFIED with direct evidence (resolver code traced, all 6 placeholder tests pass, fmt/clippy/nextest clean at 178/178) and the regression check is PRESERVED — placeholder-free configs load identically and every pre-existing load_config test still passes.

Validation notes: one benign design point outside the DoD — a `${` with no closing `}` within `PLACEHOLDER_NAME_LEN_MAX` (256) bytes is left as ordinary text rather than erroring; this does not affect any DoD scenario. The 2 skipped workspace tests are pre-existing `#[ignore]` cases requiring live KMS/DynamoDB.
