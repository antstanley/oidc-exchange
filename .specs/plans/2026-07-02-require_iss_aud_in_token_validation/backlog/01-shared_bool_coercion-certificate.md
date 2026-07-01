# Done Certificate — Task 01: shared bool-or-string coercion helper

**Task:** [01-shared_bool_coercion.md](01-shared_bool_coercion.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified

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
  - *Status:* ☐ unverified

- **O2 — Negative-space: non-`"true"`/`"false"` string, number, and null return `None`.**
  - *Claim:* `coerce_bool` returns `None` for `json!("yes")`, `json!(1)`, and `Value::Null`.
  - *Evidence to collect:* run the negative-space unit tests in `claims.rs`; confirm each asserts `None`. Trace `coerce_bool(&Value::Null)` and confirm neither the bool nor the string branch matches.
  - *Status:* ☐ unverified

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean, any bound named.
  - *Evidence to collect:* run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, and `cargo nextest run --workspace` (from `.specs/development-guidelines.md` §Definition of done) — expect all clean.
  - *Status:* ☐ unverified

- **O4 — Reviewable: the `claims` unit tests resolve each input to its documented `Option<bool>` (Reviewable).**
  - *Claim:* a reviewer runs the `claims` module tests and sees bool, `"true"`, `"false"`, and each non-coercible input map as specified.
  - *Evidence to collect:* run `cargo nextest run -p oidc-exchange-adapters` filtered to the `claims` tests; observe the bool, `"true"`, `"false"`, string, number, and null cases each PASS.
  - *Status:* ☐ unverified

## Regression check

- The `shared` module registration in `crates/adapters/src/shared/mod.rs` still exposes `discovery`, `jwks`, `token_endpoint` after adding `claims`; trace a caller of `shared::jwks` (`crates/providers/src/apple.rs:8`) and confirm it still compiles : ☐ (PRESERVED / REGRESSION)

## Residue

- The helper takes `&Value`; callers in tasks 03/04 index `claims["email_verified"]` which yields `Value::Null` for an absent key — confirmed to map to `None`, which is the intended "unknown" outcome. Not a separate obligation.

## Conclusion

VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
