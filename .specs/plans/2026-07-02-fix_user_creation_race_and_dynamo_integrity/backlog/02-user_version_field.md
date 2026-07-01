# Task 02 — Store-managed `version` field on User

**Plan:** [plan.md](../plan.md) · **Certificate:** [02-user_version_field-certificate.md](02-user_version_field-certificate.md)

**Implements:** [01-domain-model.md](../../../service/specs/01-domain-model.md) §Entities → User (the `version` field + store-managed prose), §Lifecycles (Deleted freed-identity, stated contract-first; realized by task 09), §Assumptions (non-deleted uniqueness) · [08-persistence.md](../../../service/specs/08-persistence.md) §PostgreSQL/SQLite (the `version` column sentence) · service [canonical-types.schema.json](../../../service/specs/canonical-types.schema.json) `$defs.User` · [datamodel.schema.json](../../../../schemas/datamodel.schema.json) `definitions.User`
**Depends on:** —
**Produces:** `User` carries a store-managed integer `version` that `create_user` writes as `1` and every backend persists and round-trips (missing attribute/column reads as the migration default `1`), unblocking the version-conditional `update_user` of task 08.
**Pointers:** `crates/core/src/domain/user.rs:8-25` (`User` struct; leave `NewUser`/`UserPatch` unchanged) · `crates/adapters/src/dynamo/schema.rs:12-91` (`user_to_item` write `version` as `N`, `item_to_user` read with default 1) · `crates/adapters/src/postgres/mod.rs:14-42,89-125,192-221` (migration column, `row_to_user`, `create_user` bind 1) · `crates/adapters/src/sqlite/mod.rs:14-42,100-140,211-251` (same) · `crates/test-utils/src/lib.rs:77-95` (mock `create_user`) · every other `User { … }` constructor (`crates/core/src/service/{exchange,claims}.rs`, `crates/adapters/src/webhook/mod.rs`, `crates/core/tests/{exchange,claims,user_admin}.rs`)

## Steps

- [ ] Add `pub version: u64` to `User` in `crates/core/src/domain/user.rs`; do not add it to `NewUser` or `UserPatch`.
- [ ] Update the service `canonical-types.schema.json` `$defs.User` (add `version` to `properties` and `required`) and `schemas/datamodel.schema.json` `definitions.User` (same), per the change spec's Type-changes fragment.
- [ ] DynamoDB `schema.rs`: write `version` as `AttributeValue::N` in `user_to_item`; read it in `item_to_user`, treating a missing attribute as `1`.
- [ ] Postgres/SQLite: add `version BIGINT NOT NULL DEFAULT 1` to the inline `CREATE TABLE` DDL and an idempotent `ALTER TABLE users ADD COLUMN version …` migration step for existing tables; read `version` in `row_to_user`; bind `1` in `create_user`.
- [ ] Set `version` on every remaining `User { … }` constructor (mock, providers/webhook, core service, core tests) — `1` at creation, carried through on reads.
- [ ] Update the `01-domain-model.md` User struct listing + store-managed prose, the Deleted freed-identity bullet, and the non-deleted Assumptions bullet, and the `08-persistence.md` PostgreSQL/SQLite `version`-column sentence.

## Definition of done

- [ ] `create_user` returns a `User` whose `version == 1` on DynamoDB, Postgres, and SQLite, and a subsequent read returns the same value (round-trip test per backend).
- [ ] Reading a stored user that predates the field (DynamoDB item with no `version` attribute; SQL row via the `DEFAULT 1` column) yields `version == 1` — negative-space coverage of the migration default.
- [ ] The service `canonical-types.schema.json` and `datamodel.schema.json` both list `version` in `User`'s `properties` and `required`, and the `01-domain-model.md` struct listing matches.
- [ ] Every `User { … }` constructor in the workspace sets `version`; `cargo build --workspace` has no missing-field error.
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: a reviewer runs the per-backend round-trip tests and the schema validation, observing `version == 1` on create and a schema that requires `version`.
