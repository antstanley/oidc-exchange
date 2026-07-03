# Task 05 — internal API 404 E2E

**Plan:** [plan.md](../plan.md) · **Certificate:** [05-internal_api_404_e2e-certificate.md](05-internal_api_404_e2e-certificate.md)

**Implements:** [04-http-api.md](../../../service/specs/04-http-api.md) §Routes → Internal (PATCH/DELETE/claims rows carry "404 if absent"); verifies the §Error mapping `NotFound` → 404 `not_found` end to end
**Depends on:** 01, 03, 04
**Produces:** server E2E tests proving a typo'd user id on the internal PATCH/DELETE/claims routes returns HTTP 404 `not_found` rather than 500
**Pointers:** `crates/server/tests/internal.rs` (existing internal-API E2E harness with the shared-secret setup and `body_to_json` helper), `crates/server/src/routes/internal.rs:93-146` (update/delete/claims handlers)

## Steps

- [x] Add E2E tests to `crates/server/tests/internal.rs`: PATCH `/internal/users/{unknown}` returns 404 with body `error == "not_found"`.
- [x] Add E2E tests for DELETE `/internal/users/{unknown}` and each claims route (GET, PUT, PATCH, DELETE `/internal/users/{unknown}/claims`) returning 404 `not_found`.
- [x] Assert the status is 404 (not 500) and the `error` field is `not_found`, using the existing authenticated-request harness.

## Definition of done

- [x] PATCH and DELETE on an unknown user id, and all four claims routes on an unknown user id, return HTTP 404 with `error: "not_found"` — asserted by E2E tests through the full router (negative-space: unknown id no longer yields 500 `server_error`).
- [x] The existing internal-API tests (`delete_user_returns_200`, `claims_merge_works`, and the auth tests) still pass unchanged.
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: a reviewer runs the server E2E internal-API suite and confirms a typo'd id on every mutating internal user route returns 404 `not_found`, not 500.
