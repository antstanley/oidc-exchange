# Done Certificate — Task 02: surface is_private_email on IdentityClaims

**Task:** [02-identity_claims_is_private_email.md](02-identity_claims_is_private_email.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified

> This certificate is a verification protocol for Task 02. A validating agent discharges it: for
> each obligation, collect the named evidence, run the named checks, set the Status, then derive
> the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its evidence; do
> not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 02) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names — not by assertion.

## Premises

- **P1 — Goal.** The task adds `is_private_email: Option<bool>` to `IdentityClaims`, updates the
  canonical schema and 01-domain-model prose, and updates every constructor so the workspace compiles.
- **P2 — Obligations.** Done iff O1…O4 all hold; O4 is the Reviewable item.
- **P3 — Invariants.** Must not break the `IdentityProvider` port (`crates/core/src/ports/identity_provider.rs:12`, returns `IdentityClaims` unchanged) or the existing consumers reading `IdentityClaims` fields (e.g. `crates/core/src/service/exchange.rs` reading `email`/`email_verified`).

## Obligations

- **O1 — `IdentityClaims` has `is_private_email: Option<bool>` and the workspace compiles with every constructor supplying it.**
  - *Claim:* the struct at `crates/core/src/domain/token.rs:74-81` gains the field, and no constructor uses a `..Default`/shim — every call site names it.
  - *Evidence to collect:* read `token.rs` and confirm the field is present. Grep `IdentityClaims {` across `crates/` and confirm each of the constructors listed in the task `Pointers` (`adapters/src/oidc/mod.rs:157`, `providers/src/apple.rs:280`, `test-utils/src/lib.rs:414` & `:451`, `core/tests/exchange.rs` six sites) supplies `is_private_email`. Run `cargo build --workspace` — expect success with no missing-field error.
  - *Checks:* confirm `apple.rs` sets `None` here (task 04 populates it) — an early populate is acceptable but the field must be present.
  - *Status:* ☐ unverified

- **O2 — Schema and prose describe `is_private_email`, updated together.**
  - *Claim:* `$defs/IdentityClaims` in `.specs/service/specs/canonical-types.schema.json` carries an `is_private_email` property (`["boolean","null"]`) and the §"Token types" bullet in `.specs/service/specs/01-domain-model.md` names it.
  - *Evidence to collect:* read both files; confirm the schema property and the prose bullet exist and that 01-domain-model's `**Date:**` was bumped.
  - *Status:* ☐ unverified

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean.
  - *Evidence to collect:* run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, and `cargo nextest run --workspace` (from `.specs/development-guidelines.md` §Definition of done) — expect all clean.
  - *Status:* ☐ unverified

- **O4 — Reviewable: the workspace builds with no constructor un-updated and the schema/prose name the field (Reviewable).**
  - *Claim:* a reviewer runs the workspace test suite and confirms the schema and 01-domain-model prose describe `is_private_email`.
  - *Evidence to collect:* run `cargo nextest run --workspace` — expect green; open the schema and 01-domain-model page and confirm the field is documented.
  - *Status:* ☐ unverified

## Regression check

- `crates/core/src/service/exchange.rs` reads `claims.email` and `claims.email_verified` from `IdentityClaims`; trace that the added field does not alter those reads — expect the allowlist path unchanged : ☐ (PRESERVED / REGRESSION)
- `crates/test-utils/src/lib.rs` mock provider constructs `IdentityClaims`; trace that tests using it still compile and pass : ☐ (PRESERVED / REGRESSION)

## Residue

- Task 04 replaces the `None` placeholder in `apple.rs` with coercion; this task only requires the field to be present and defaulted, not populated.

## Conclusion

VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
