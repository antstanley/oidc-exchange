# Done Certificate — Task 05: internal API 404 E2E

**Task:** [05-internal_api_404_e2e.md](05-internal_api_404_e2e.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

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
  - *Status:* ☑ SATISFIED — six new tests at `crates/server/tests/internal.rs:436-577` (`update_user_unknown_id_returns_404_not_found`, `delete_user_unknown_id_returns_404_not_found`, `get_claims_unknown_id_returns_404_not_found`, `set_claims_unknown_id_returns_404_not_found`, `merge_claims_unknown_id_returns_404_not_found`, `clear_claims_unknown_id_returns_404_not_found`) each assert `StatusCode::NOT_FOUND` and `body["error"] == "not_found"` via `body_to_json` with the `Bearer TEST_SECRET` header; all six PASS under `cargo nextest run -p oidc-exchange --test internal`. Check — router resolution: `build_test_app()` (internal.rs:22-50) merges `internal_routes(state)`, which resolves to `internal::router` (routes/mod.rs:25-27), whose handlers `?`-propagate the service's `Error::NotFound` into `ApiError` mapped to 404/`not_found` at `crates/server/src/error.rs:91-95` — the full-router path, not a direct service call. Check — absent id: `MockRepository::new()` starts with an empty `users` HashMap (test-utils), each test builds a fresh app and creates no user, so `usr_does_not_exist` (a named constant) is absent.

- **O2 — Existing internal-API tests still pass unchanged.**
  - *Claim:* `delete_user_returns_200`, `claims_merge_works`, and the auth tests remain green.
  - *Evidence to collect:* run the full `crates/server/tests/internal.rs` suite — expect PASS with no edits to the existing tests' assertions.
  - *Status:* ☑ SATISFIED — `cargo nextest run -p oidc-exchange --test internal`: 14/14 PASS, including `delete_user_returns_200`, `claims_merge_works`, `internal_auth_rejects_missing_auth`, `internal_auth_rejects_wrong_secret`, `internal_auth_passes_with_correct_secret`, `internal_auth_rejects_empty_configured_secret_even_with_empty_bearer_token`. `jj diff` on `internal.rs` is pure addition (+149/-0, appended after line 426) — no existing test or assertion touched.

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean.
  - *Evidence to collect:* run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` (per `.specs/development-guidelines.md` §Definition of done) — expect clean.
  - *Status:* ☑ SATISFIED — `cargo fmt --check` exit 0; `cargo clippy --workspace -- -D warnings` exit 0; `cargo nextest run --workspace` → 307 passed, 0 failed, 27 skipped. The unknown id is the named constant `UNKNOWN_USER_ID` (no magic literal).

- **O4 — Reviewable: a typo'd id on every mutating internal user route returns 404 `not_found`, not 500.**
  - *Claim:* a reviewer running the server E2E internal-API suite sees 404 `not_found` on PATCH, DELETE, and all four claims routes for an unknown id.
  - *Evidence to collect:* run `cargo nextest run -p oidc-exchange-server` (or the workspace suite) and read the six unknown-id assertions; confirm none returns 500 `server_error`.
  - *Status:* ☑ SATISFIED — exercised as the reviewer would: ran `cargo nextest run -p oidc-exchange --test internal` (the server crate's package name is `oidc-exchange`, not `oidc-exchange-server`) — all six unknown-id tests PASS; each additionally asserts `assert_ne!(status, StatusCode::INTERNAL_SERVER_ERROR)` and `error == "not_found"`, so no route returns 500 `server_error`. PATCH, DELETE, and all four claims verbs (GET/PUT/PATCH/DELETE) are covered.

## Regression check

- `delete_user_returns_200` (`crates/server/tests/internal.rs:327-...`) exercises DELETE on an existing user → expect it still returns 200 after the unknown-id 404 tests are added : ☑ PRESERVED — PASS in the discharged run; the diff adds only new tests, no product code changed by this task.
- `claims_merge_works` (`:241-...`) exercises the claims routes on an existing user → expect the positive path still returns merged claims : ☑ PRESERVED — PASS in the discharged run; positive-path assertions unchanged.

## Residue

- This task adds only tests; the 404 behaviour it verifies is produced by Tasks 01, 03, and 04. If a route still returns 500, the defect is upstream (a missing `NotFound` in the service), not in these tests.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O4 all SATISFIED with evidence in hand — six new full-router E2E tests assert 404/`not_found` (and explicitly not 500) for PATCH/DELETE user and all four claims verbs on an absent id, the 14-test internal suite and the 307-test workspace suite pass, fmt/clippy are clean, and both named regression callers (`delete_user_returns_200`, `claims_merge_works`) are PRESERVED on a purely additive diff.
