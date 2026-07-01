# Done Certificate — Task 04: claims not_found on unknown id

**Task:** [04-claims_not_found_on_unknown_id.md](04-claims_not_found_on_unknown_id.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified

> This certificate is a verification protocol for Task 04. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 04) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names — not by assertion.

## Premises

- **P1 — Goal.** The four claims operations return `Error::NotFound` on an unknown user id, agreeing with GET.
- **P2 — Obligations.** Done iff O1…O4 all hold. One Oi per definition-of-done item, in DoD order; O4 is the Reviewable item.
- **P3 — Invariants.** Must not change the positive-path claims behaviour (`admin_get_claims`/`admin_set_claims`/`admin_merge_claims`/`admin_clear_claims` for an existing user).

## Obligations

- **O1 — All four claims operations return `Error::NotFound` for an unknown user id.**
  - *Claim:* the `get_user_by_id` miss branch in each of `admin_get_claims`, `admin_set_claims`, `admin_merge_claims`, `admin_clear_claims` returns `Error::NotFound { detail }`, not `InvalidRequest`.
  - *Evidence to collect:* read `crates/core/src/service/user_admin.rs` at the four pre-checks (`:87-97`, `:100-111`, `:127-138`, `:157-164`); run core tests asserting each returns `Err(Error::NotFound { .. })` on an unknown id — expect PASS.
  - *Checks:* resolve `Error::NotFound` to the Task-01 variant; confirm no `InvalidRequest` remains in these four pre-checks.
  - *Status:* ☐ unverified

- **O2 — Existing positive-path claims tests still pass unchanged.**
  - *Claim:* `admin_merge_claims_preserves_existing`, `admin_set_claims_replaces_entirely`, `admin_clear_claims_empties_map` remain green.
  - *Evidence to collect:* run those three tests in `crates/core/tests/user_admin.rs` — expect PASS with no edits to their assertions.
  - *Status:* ☐ unverified

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean.
  - *Evidence to collect:* run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` (per `.specs/development-guidelines.md` §Definition of done) — expect clean.
  - *Status:* ☐ unverified

- **O4 — Reviewable: unknown id yields `NotFound` on all four operations; happy-path stays green.**
  - *Claim:* a reviewer running the claims core tests sees `NotFound` on unknown-id cases and green happy-path tests.
  - *Evidence to collect:* run the `user_admin` claims tests and confirm the four unknown-id assertions return `NotFound` while the three positive-path tests pass.
  - *Status:* ☐ unverified

## Regression check

- The four claims service functions are also called by the internal claims routes (`crates/server/src/routes/internal.rs:114-146`) → the error-type switch changes the miss from 400 to 404 (intended, verified end to end by Task 05); trace one positive call (existing user) and confirm the success path is unchanged : ☐ (PRESERVED / REGRESSION)

## Residue

- The full-stack 404 verification for these routes is Task 05, not an obligation here.

## Conclusion

VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
