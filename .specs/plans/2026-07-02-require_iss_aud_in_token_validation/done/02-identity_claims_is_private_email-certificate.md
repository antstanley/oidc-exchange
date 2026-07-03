# Done Certificate — Task 02: surface is_private_email on IdentityClaims

**Task:** [02-identity_claims_is_private_email.md](02-identity_claims_is_private_email.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

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
  - *Status:* ☑ SATISFIED — field present at `crates/core/src/domain/token.rs:82` with doc comment. Grep of `IdentityClaims {` across `crates/` finds exactly the pointed sites (struct def + 10 constructors: `adapters/src/oidc/mod.rs:157`, `providers/src/apple.rs:280`, `test-utils/src/lib.rs:414` & `:452`, `core/tests/exchange.rs:264/:309/:335/:361/:392/:506`) and each supplies `is_private_email: None` in the diff; no `..Default`/struct-update shim anywhere. `apple.rs` sets `None` as required. Compilation: `cargo clippy --workspace -- -D warnings` (full type-check) and `cargo nextest run --workspace` (builds all test binaries) both succeed. Note: plain `cargo build --workspace` fails at the *link* stage of `oidc-exchange-python` (pyo3 `extension-module` cdylib needs maturin's linker args on macOS) — a pre-existing environment artifact untouched by this diff, not a missing-field error.

- **O2 — Schema and prose describe `is_private_email`, updated together.**
  - *Claim:* `$defs/IdentityClaims` in `.specs/service/specs/canonical-types.schema.json` carries an `is_private_email` property (`["boolean","null"]`) and the §"Token types" bullet in `.specs/service/specs/01-domain-model.md` names it.
  - *Evidence to collect:* read both files; confirm the schema property and the prose bullet exist and that 01-domain-model's `**Date:**` was bumped.
  - *Status:* ☑ SATISFIED — `canonical-types.schema.json` `$defs/IdentityClaims` now carries `"is_private_email": { "type": ["boolean","null"], "description": "Apple private-relay flag, coerced bool-or-string like email_verified; null for non-Apple providers." }`; the §"Token types" `IdentityClaims` bullet in `01-domain-model.md:76-78` lists `is_private_email` (Apple private-relay flag; `None` for other providers); the page `**Date:**` bumped 2026-06-24 → 2026-07-02. Both changed in the same diff as the type change.

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean.
  - *Evidence to collect:* run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, and `cargo nextest run --workspace` (from `.specs/development-guidelines.md` §Definition of done) — expect all clean.
  - *Status:* ☑ SATISFIED — `cargo fmt --check` exit 0; `cargo clippy --workspace -- -D warnings` finished clean; `cargo nextest run --workspace` → 223 passed, 0 failed, 10 skipped. No new bounds introduced, so the named-constant limit is vacuously met; the task adds a field + `None` at existing sites, so no new validation path requiring a negative-space test.

- **O4 — Reviewable: the workspace builds with no constructor un-updated and the schema/prose name the field (Reviewable).**
  - *Claim:* a reviewer runs the workspace test suite and confirms the schema and 01-domain-model prose describe `is_private_email`.
  - *Evidence to collect:* run `cargo nextest run --workspace` — expect green; open the schema and 01-domain-model page and confirm the field is documented.
  - *Status:* ☑ SATISFIED — exercised as this validation: `cargo nextest run --workspace` green (223/223 passed); grep confirms no `IdentityClaims { … }` constructor lacks `is_private_email`; the schema `$defs/IdentityClaims` and the 01-domain-model §"Token types" bullet both name and describe the field.

## Regression check

- `crates/core/src/service/exchange.rs` reads `claims.email` and `claims.email_verified` from `IdentityClaims`; trace that the added field does not alter those reads — expect the allowlist path unchanged : ☑ PRESERVED — the diff does not touch `exchange.rs`; the added `Option<bool>` field is additive and the allowlist tests in `crates/core/tests/exchange.rs` (subdomain accept/reject, exact-domain, not-allowed, no-email sites at :264–:506) all pass in the nextest run.
- `crates/test-utils/src/lib.rs` mock provider constructs `IdentityClaims`; trace that tests using it still compile and pass : ☑ PRESERVED — both mock constructors (`lib.rs:414`, `:452`) supply `is_private_email: None`; the full workspace suite that consumes the mock compiles and passes (223/223).

## Residue

- Task 04 replaces the `None` placeholder in `apple.rs` with coercion; this task only requires the field to be present and defaulted, not populated.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O4 all SATISFIED with collected evidence (field present, all 10 constructors updated with no shim, schema + 01-domain-model prose changed together with the date bumped, fmt/clippy/nextest all clean at 223/223) and both regression traces PRESERVED; the only anomaly — `cargo build --workspace` failing at the pyo3 `extension-module` link stage — is a pre-existing environment artifact unrelated to this diff, with full compilation proven by clippy and nextest.
