# Task 08 — Version-conditional update_user on every backend

**Plan:** [plan.md](../plan.md) · **Certificate:** [08-versioned_update_user-certificate.md](08-versioned_update_user-certificate.md)

**Implements:** [08-persistence.md](../../../service/specs/08-persistence.md) §DynamoDB (version-conditional `update_user`) · [02-ports-and-adapters.md](../../../service/specs/02-ports-and-adapters.md) §UserRepository (the `update_user` version-atomicity guarantee, stated in task 01, realized here)
**Depends on:** 02
**Produces:** `update_user` writes conditionally on the integer `version` it read and increments it on every backend, retrying the read-modify-write on a version conflict up to a named bound, so two racing patches serialize and a lost update cannot silently revert a concurrent status change.
**Pointers:** `crates/adapters/src/dynamo/mod.rs:116-153` (`update_user`) · `crates/adapters/src/postgres/mod.rs:224-270` (`update_user`) · `crates/adapters/src/sqlite/mod.rs:254-307` (`update_user`) · `crates/test-utils/src/lib.rs:97-...` (mock `update_user`, increment version) · a named retry-attempt constant per adapter

## Steps

- [ ] DynamoDB: store `version` on the item (from task 02) and write with `condition_expression("version = :read_version OR attribute_not_exists(version)")`, setting `version = read + 1`; on a conditional-check failure, re-read and retry up to a named attempt bound, then error.
- [ ] Postgres/SQLite: change the `UPDATE` to `SET version = version + 1 … WHERE id = $1 AND version = $2`; when zero rows are affected, re-read and retry up to the named bound, then error.
- [ ] Add a named retry-attempt constant (e.g. `UPDATE_MAX_ATTEMPTS`) in each adapter and reference it by name — no magic loop bound.
- [ ] Increment `version` in `MockRepository::update_user` so the mock matches the durable backends' semantics.
- [ ] Update the `08-persistence.md` §DynamoDB version-conditional-`update_user` sentence.

## Definition of done

- [ ] A test simulating a racing suspend + claims patch (two reads at the same `version`, two writes) ends with the user `Suspended` — one write wins, the other retries against the new version — on DynamoDB, Postgres, and SQLite.
- [ ] Negative-space test: a patch whose read `version` can never match (the row keeps changing) exhausts the retry budget and returns an error rather than looping unbounded or silently overwriting.
- [ ] Each adapter's retry bound is a named constant referenced by name; the read-modify-write increments `version` by exactly one per successful write.
- [ ] `MockRepository::update_user` increments `version`, keeping test semantics aligned with the durable backends.
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: a reviewer runs the racing-patch tests on all three backends and observes the suspend surviving the concurrent claims patch.
