# Task 01 — shared bool-or-string coercion helper

**Plan:** [plan.md](../plan.md) · **Certificate:** [01-shared_bool_coercion-certificate.md](01-shared_bool_coercion-certificate.md)

**Implements:** [05-provider-system.md](../../../service/specs/05-provider-system.md) §"Tiers, Tier 2 Apple" (the bool-or-string coercion the change spec places in `adapters/shared`) and its Decision *Coercion is shared, not Apple-only*.
**Depends on:** —
**Produces:** a `coerce_bool` helper in `crates/adapters/src/shared` that maps a JSON value to `Option<bool>` — `true`/`false` bools pass through, the JSON strings `"true"`/`"false"` coerce to `Some(true)`/`Some(false)`, everything else (numbers, other strings, null, absent) yields `None` — with unit tests covering each case.
**Pointers:** new module `crates/adapters/src/shared/claims.rs`; register it in `crates/adapters/src/shared/mod.rs` (currently `discovery`, `jwks`, `token_endpoint`); the module is already public via `crates/adapters/src/lib.rs:8` (`pub mod shared`).

## Steps

- [ ] Add `crates/adapters/src/shared/claims.rs` with `pub fn coerce_bool(value: &serde_json::Value) -> Option<bool>` that returns `value.as_bool()` when the value is a JSON bool, else matches `value.as_str()` on `"true"`/`"false"`, else `None`.
- [ ] Register `pub mod claims;` in `crates/adapters/src/shared/mod.rs`.
- [ ] Add unit tests in the module: `true`/`false` bools, `"true"`/`"false"` strings, and non-coercible inputs (a number, an arbitrary string like `"yes"`, `null`, and an absent-key `Value::Null`) each mapping as specified.
- [ ] Add at least two meaningful assertions to the helper's contract via the tests (both the positive coercion and the `None` fallback are asserted).

## Definition of done

- [ ] `coerce_bool` maps JSON bool and the strings `"true"`/`"false"` to the matching `Some(bool)`, and every other input to `None`.
- [ ] Negative-space test: a non-`"true"`/`"false"` string, a number, and `null` all return `None` (the coercion never guesses).
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: a reviewer runs the `claims` module's unit tests and sees bool, `"true"`, `"false"`, and each non-coercible input resolve to the documented `Option<bool>`.
