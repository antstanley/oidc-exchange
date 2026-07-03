# Task 03 — Exchange flow: conflict on JIT create → re-lookup

**Plan:** [plan.md](../plan.md) · **Certificate:** [03-exchange_re_lookup-certificate.md](03-exchange_re_lookup-certificate.md)

**Implements:** [03-service-flows.md](../../../service/specs/03-service-flows.md) §Token exchange (step 3: `Conflict` on JIT create → re-lookup and continue) · [02-ports-and-adapters.md](../../../service/specs/02-ports-and-adapters.md) §Mock adapters (`MockRepository` gains non-deleted uniqueness so the race is exercisable)
**Depends on:** 01, 02
**Produces:** A first login racing a concurrent first login for the same subject returns a token instead of a `500`: on `create_user` → `Conflict`, the exchange re-runs `get_user_by_external_id` and continues on the found-user branch, re-applying the suspended-status check.
**Pointers:** `crates/core/src/service/exchange.rs:131-138` (wrap `create_user`) · `crates/core/src/service/exchange.rs:85-95` (found-user branch to re-enter) · `crates/test-utils/src/lib.rs:77-95` (mock `create_user` — enforce non-deleted `(provider, external_id)` uniqueness returning `Conflict`; exclude deleted from `get_user_by_external_id:64-75`) · `crates/core/tests/exchange.rs`

## Steps

- [x] In `exchange.rs`, match the `create_user` result: on `Err(Error::Conflict { .. })`, re-run `get_user_by_external_id(&claims.subject, &request.provider)`, take the returned user, and re-apply the `status != Active → UserSuspended` check; on any other `Err`, propagate.
- [x] If the re-lookup returns `None` (the winner's row is somehow absent), surface a distinct error rather than panicking — keep the branch total.
- [x] Do not emit a second create or a `UserCreated` audit event on the losing racer; leave the `(audited UserCreated)` annotation in the 03-service-flows bullet untouched (it composes with the audit-emission change spec).
- [x] Make `MockRepository::create_user` reject a duplicate live `(provider, external_id)` with `Error::Conflict`, and exclude `Deleted` users from `get_user_by_external_id`, so the core test can drive the race deterministically.
- [x] Update the `03-service-flows.md` exchange step-3 bullet with the conflict → re-lookup prose.

## Definition of done

- [x] A core test drives two `exchange` calls for one subject against a shared mock where the second `create_user` conflicts, and both return a `TokenResponse` — the second via the re-lookup path — with no `500`/`StoreError`.
- [x] The re-lookup path re-applies the suspended check: a test where the winning user is `Suspended` returns `UserSuspended`, not a token.
- [x] Negative-space test: a non-`Conflict` `create_user` error (e.g. `StoreError`) still propagates as an error, not a silent re-lookup.
- [x] `MockRepository` enforces non-deleted `(provider, external_id)` uniqueness and excludes deleted users from external-id lookup, matching the durable backends.
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: a reviewer runs the concurrent-first-login exchange test and observes two tokens issued and exactly one user created.
