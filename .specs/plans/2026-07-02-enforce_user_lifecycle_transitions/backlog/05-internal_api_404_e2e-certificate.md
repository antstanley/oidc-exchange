# Done Certificate — Task 05: internal API 404 E2E

**Task:** [05-internal_api_404_e2e.md](05-internal_api_404_e2e.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified

> This certificate is a verification protocol for Task 05. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 05) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names — not by assertion.

## Premises

- **P1 — Goal.** Server E2E tests prove a typo'd user id on the internal PATCH/DELETE/claims routes returns HTTP 404 `not_found` rather than 500.
- **P2 — Obligations.** Done iff O1…O4 all hold. One Oi per definition-of-done item, in DoD order; O4 is the Reviewable item.
- **P3 — Invariants.** Must not break the existing internal-API tests (`delete_user_returns_200`, `claims_merge_works`, and the auth tests) in `crates/server/tests/internal.rs`.

## Obligations

- **O1 — PATCH/DELETE and all four claims routes on an unknown id return 404 `not_found` through the full router.**
  - *Claim:* PATCH `/internal/users/{unknown}`, DELETE `/internal/users/{unknown}`, and GET/PUT/PATCH/DELETE `/internal/users/{unknown}/claims` each respond with status 404 and body `error == "not_found"`.
  - *Evidence to collect:* read the new tests in `crates/server/tests/internal.rs`; run them — expect PASS, each asserting `StatusCode::NOT_FOUND` and `body["error"] == "not_found"` via the authenticated request harness and `body_to_json`.
  - *Checks:* confirm the tests drive the real `internal_routes` router (not the service directly) so the `map_domain_error` → 404 mapping from Task 01 is exercised end to end; confirm the id used is absent from the seeded repo.
  - *Status:* ☐ unverified

- **O2 — Existing internal-API tests still pass unchanged.**
  - *Claim:* `delete_user_returns_200`, `claims_merge_works`, and the auth tests remain green.
  - *Evidence to collect:* run the full `crates/server/tests/internal.rs` suite — expect PASS with no edits to the existing tests' assertions.
  - *Status:* ☐ unverified

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean.
  - *Evidence to collect:* run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` (per `.specs/development-guidelines.md` §Definition of done) — expect clean.
  - *Status:* ☐ unverified

- **O4 — Reviewable: a typo'd id on every mutating internal user route returns 404 `not_found`, not 500.**
  - *Claim:* a reviewer running the server E2E internal-API suite sees 404 `not_found` on PATCH, DELETE, and all four claims routes for an unknown id.
  - *Evidence to collect:* run `cargo nextest run -p oidc-exchange-server` (or the workspace suite) and read the six unknown-id assertions; confirm none returns 500 `server_error`.
  - *Status:* ☐ unverified

## Regression check

- `delete_user_returns_200` (`crates/server/tests/internal.rs:327-...`) exercises DELETE on an existing user → expect it still returns 200 after the unknown-id 404 tests are added : ☐ (PRESERVED / REGRESSION)
- `claims_merge_works` (`:241-...`) exercises the claims routes on an existing user → expect the positive path still returns merged claims : ☐ (PRESERVED / REGRESSION)

## Residue

- This task adds only tests; the 404 behaviour it verifies is produced by Tasks 01, 03, and 04. If a route still returns 500, the defect is upstream (a missing `NotFound` in the service), not in these tests.

## Conclusion

VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
