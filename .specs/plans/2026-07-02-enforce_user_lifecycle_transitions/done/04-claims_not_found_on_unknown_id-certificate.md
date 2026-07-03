# Done Certificate — Task 04: claims not_found on unknown id

**Task:** [04-claims_not_found_on_unknown_id.md](04-claims_not_found_on_unknown_id.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

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
  - *Status:* ☑ SATISFIED — all four pre-checks (`user_admin.rs:139`, `:156`, `:183`, `:209`) return `Error::NotFound { detail: "user not found: {id}" }`; `Error::NotFound` resolves to the Task-01 variant at `crates/core/src/error.rs:31`; no `InvalidRequest` remains in the four pre-checks (remaining mentions in the file are lifecycle-transition logic, out of scope). New tests `admin_{get,set,merge,clear}_claims_unknown_id_returns_not_found` all PASS.

- **O2 — Existing positive-path claims tests still pass unchanged.**
  - *Claim:* `admin_merge_claims_preserves_existing`, `admin_set_claims_replaces_entirely`, `admin_clear_claims_empties_map` remain green.
  - *Evidence to collect:* run those three tests in `crates/core/tests/user_admin.rs` — expect PASS with no edits to their assertions.
  - *Status:* ☑ SATISFIED — `admin_merge_claims_preserves_existing`, `admin_set_claims_replaces_entirely`, `admin_clear_claims_empties_map` all PASS; the diff to `crates/core/tests/user_admin.rs` is pure additions (new unknown-id tests), no existing assertions edited.

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean.
  - *Evidence to collect:* run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` (per `.specs/development-guidelines.md` §Definition of done) — expect clean.
  - *Status:* ☑ SATISFIED — `cargo fmt --check` clean (exit 0); `cargo clippy --workspace -- -D warnings` clean; `cargo nextest run --workspace` 301 passed / 0 failed / 27 skipped. No new limits introduced, so no named-constant work applies.

- **O4 — Reviewable: unknown id yields `NotFound` on all four operations; happy-path stays green.**
  - *Claim:* a reviewer running the claims core tests sees `NotFound` on unknown-id cases and green happy-path tests.
  - *Evidence to collect:* run the `user_admin` claims tests and confirm the four unknown-id assertions return `NotFound` while the three positive-path tests pass.
  - *Status:* ☑ SATISFIED — `cargo nextest run -p oidc-exchange-core --test user_admin -E 'test(claims)'` ran 7 tests, all PASS: four unknown-id `NotFound` assertions (each also asserts the error is not `InvalidRequest`; set/merge/clear additionally assert no user row was written) and the three positive-path tests.

## Regression check

- The four claims service functions are also called by the internal claims routes (`crates/server/src/routes/internal.rs:114-146`) → the error-type switch changes the miss from 400 to 404 (intended, verified end to end by Task 05); trace one positive call (existing user) and confirm the success path is unchanged : ☑ PRESERVED — the four internal routes (`internal.rs:114-146`) propagate the service error via `?`; the diff only changes the `.ok_or_else` miss branch, so an existing user (`Some(user)`) never reaches the changed code; positive-path tests green. `Error::NotFound` maps to 404 at `crates/server/src/error.rs:91-95` (the intended 400→404 shift on a miss, verified end to end by Task 05).

## Residue

- The full-stack 404 verification for these routes is Task 05, not an obligation here.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: All four claims pre-checks now return `Error::NotFound` with the unchanged `user not found: {id}` message, proven by four new passing unknown-id core tests; the three positive-path claims tests pass unedited; fmt/clippy/full-workspace nextest (301/301) are clean; the internal-route success path is preserved with the miss now mapping to 404 as intended.
