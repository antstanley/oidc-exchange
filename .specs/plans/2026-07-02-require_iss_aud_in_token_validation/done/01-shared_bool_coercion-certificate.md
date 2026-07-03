# Done Certificate — Task 01: shared bool-or-string coercion helper

**Task:** [01-shared_bool_coercion.md](01-shared_bool_coercion.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

> This certificate is a verification protocol for Task 01. A validating agent discharges it: for
> each obligation, collect the named evidence, run the named checks, set the Status, then derive
> the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its evidence; do
> not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 01) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** The task produces a `coerce_bool` helper in `crates/adapters/src/shared` that
  maps a JSON value to `Option<bool>`: bools pass through, `"true"`/`"false"` strings coerce, all
  else is `None`.
- **P2 — Obligations.** The task is done iff O1…O4 all hold; O4 is the Reviewable item.
- **P3 — Invariants.** Must not disturb the existing `shared` modules (`discovery`, `jwks`,
  `token_endpoint`); the change is additive — a new `claims` module and its `mod.rs` registration.

## Obligations

- **O1 — `coerce_bool` maps bool and `"true"`/`"false"` to `Some(bool)`, else `None`.**
  - *Claim:* `coerce_bool(&json!(true)) == Some(true)`, `coerce_bool(&json!("false")) == Some(false)`, and `coerce_bool(&json!("yes")) == None`.
  - *Evidence to collect:* read `crates/adapters/src/shared/claims.rs`; confirm the body returns `value.as_bool()` for a JSON bool and matches `value.as_str()` on `"true"`/`"false"` before falling through to `None`. Run the module's unit tests (`cargo nextest run -p oidc-exchange-adapters claims` or the workspace run) — expect the bool and string cases PASS.
  - *Checks:* resolve `as_bool`/`as_str` to `serde_json::Value` methods, not a local shadow.
  - *Evidence:* `crates/adapters/src/shared/claims.rs:15-24` — body returns `Some(b)` from `value.as_bool()` when the value is a JSON bool, then matches `value.as_str()` on `"true"`/`"false"`, falling through to `None`. Ran `cargo nextest run -p oidc-exchange-adapters -E 'test(claims)'` → 8/8 PASS, including `coerces_json_bool_true`/`_false` and `coerces_string_true`/`_false`. Check: no local `as_bool`/`as_str` defined in the module; both calls are on the `&serde_json::Value` receiver and resolve to `serde_json::Value`'s inherent methods — no shadowing.
  - *Status:* SATISFIED

- **O2 — Negative-space: non-`"true"`/`"false"` string, number, and null return `None`.**
  - *Claim:* `coerce_bool` returns `None` for `json!("yes")`, `json!(1)`, and `Value::Null`.
  - *Evidence to collect:* run the negative-space unit tests in `claims.rs`; confirm each asserts `None`. Trace `coerce_bool(&Value::Null)` and confirm neither the bool nor the string branch matches.
  - *Evidence:* tests `non_coercible_string_yields_none` (`json!("yes")`), `non_coercible_number_yields_none` (`json!(1)`), `null_yields_none` (`Value::Null`), and `absent_key_yields_none` (`claims["email_verified"]` on a map without the key) each assert `None` → all PASS. Trace of `coerce_bool(&Value::Null)`: `Null.as_bool()` → `None` (bool branch skipped), `Null.as_str()` → `None` (string branch's `Some(...)` arms unmatched) → `_ => None`.
  - *Status:* SATISFIED

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean, any bound named.
  - *Evidence to collect:* run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, and `cargo nextest run --workspace` (from `.specs/development-guidelines.md` §Definition of done) — expect all clean.
  - *Evidence:* `cargo fmt --check` → exit 0 (clean). `cargo clippy --workspace -- -D warnings` → Finished, exit 0, no warnings. `cargo nextest run --workspace` → 223 tests run: 223 passed, 10 skipped. No numeric bounds introduced, so no named-constant obligation arises.
  - *Status:* SATISFIED

- **O4 — Reviewable: the `claims` unit tests resolve each input to its documented `Option<bool>` (Reviewable).**
  - *Claim:* a reviewer runs the `claims` module tests and sees bool, `"true"`, `"false"`, and each non-coercible input map as specified.
  - *Evidence to collect:* run `cargo nextest run -p oidc-exchange-adapters` filtered to the `claims` tests; observe the bool, `"true"`, `"false"`, string, number, and null cases each PASS.
  - *Evidence:* exercised as the reviewer would: `cargo nextest run -p oidc-exchange-adapters -E 'test(claims)'` → 8 tests, 8 PASS (`coerces_json_bool_true`, `coerces_json_bool_false`, `coerces_string_true`, `coerces_string_false`, `non_coercible_string_yields_none`, `non_coercible_number_yields_none`, `null_yields_none`, `absent_key_yields_none`) — each input resolves to its documented `Option<bool>`.
  - *Status:* SATISFIED

## Regression check

- The `shared` module registration in `crates/adapters/src/shared/mod.rs` still exposes `discovery`, `jwks`, `token_endpoint` after adding `claims`; trace a caller of `shared::jwks` (`crates/providers/src/apple.rs:8`) and confirm it still compiles : PRESERVED — `mod.rs` now reads `claims`, `discovery`, `jwks`, `token_endpoint` (diff is one added line); `crates/providers/src/apple.rs:8` still imports `oidc_exchange_adapters::shared::jwks::JwksCache`, the workspace compiles under clippy `-D warnings`, and the apple provider tests (e.g. `apple::tests::exchange_and_validate_flow`) pass in the workspace run. The diff touches no existing module.

## Residue

- The helper takes `&Value`; callers in tasks 03/04 index `claims["email_verified"]` which yields `Value::Null` for an absent key — confirmed to map to `None`, which is the intended "unknown" outcome. Not a separate obligation.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O4 all SATISFIED with evidence in hand — the 8 `claims` unit tests pass, fmt/clippy/workspace-nextest (223/223) are clean, `as_bool`/`as_str` resolve to `serde_json::Value` methods with no shadowing — and the additive change leaves the `shared::jwks` caller in `apple.rs` PRESERVED.
