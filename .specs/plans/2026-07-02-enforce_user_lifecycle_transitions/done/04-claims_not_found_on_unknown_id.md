# Task 04 — claims not_found on unknown id

**Plan:** [plan.md](../plan.md) · **Certificate:** [04-claims_not_found_on_unknown_id-certificate.md](04-claims_not_found_on_unknown_id-certificate.md)

**Implements:** [03-service-flows.md](../../../service/specs/03-service-flows.md) §Admin operations (claims rows move from `InvalidRequest` to `NotFound` for unknown ids); [04-http-api.md](../../../service/specs/04-http-api.md) §Routes → Internal (claims rows gain the "404 if absent" annotation)
**Depends on:** 01
**Produces:** the four claims operations return `Error::NotFound` on an unknown user id, agreeing with GET
**Pointers:** `crates/core/src/service/user_admin.rs:87-97` (`admin_get_claims`), `:100-111` (`admin_set_claims`), `:127-138` (`admin_merge_claims`), `:157-164` (`admin_clear_claims`); `crates/core/src/error.rs` (`Error::NotFound`, from task 01)

## Steps

- [x] Switch the four claims pre-checks from `Error::InvalidRequest { reason }` to `Error::NotFound { detail }` on a `get_user_by_id` miss, keeping the same `user not found: {id}` message.
- [x] Add core tests asserting each of the four claims operations returns `Error::NotFound` for an unknown id.

## Definition of done

- [x] `admin_get_claims`, `admin_set_claims`, `admin_merge_claims`, and `admin_clear_claims` each return `Error::NotFound` for an unknown user id, asserted by core tests (negative-space: unknown id on every claims operation).
- [x] The existing positive-path claims tests (`admin_merge_claims_preserves_existing`, `admin_set_claims_replaces_entirely`, `admin_clear_claims_empties_map`) still pass unchanged.
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: a reviewer runs the claims core tests and confirms an unknown id yields `NotFound` on all four operations while the existing happy-path tests remain green.
