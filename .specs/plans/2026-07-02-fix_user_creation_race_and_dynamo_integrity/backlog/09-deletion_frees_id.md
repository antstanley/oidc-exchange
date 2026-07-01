# Task 09 — Deletion frees the external id for re-registration

**Plan:** [plan.md](../plan.md) · **Certificate:** [09-deletion_frees_id-certificate.md](09-deletion_frees_id-certificate.md)

**Implements:** [08-persistence.md](../../../service/specs/08-persistence.md) §DynamoDB (guard delete on the `Deleted` transition), §PostgreSQL/SQLite (partial unique index excluding deleted rows; deleted rows excluded from lookup) · [01-domain-model.md](../../../service/specs/01-domain-model.md) §Lifecycles (Deleted freed-identity, stated in task 02, realized here) · [table-design.json](../../../../schemas/dynamodb/table-design.json) (guard removed on delete)
**Depends on:** 06, 07, 08
**Produces:** After a delete, `(provider, external_id)` is free: `get_user_by_external_id` returns nothing for that identity and a later first login re-registers as a brand-new user with no claims or sessions carried over — on DynamoDB (the guard item is removed) and on SQL (a partial unique index excludes deleted rows and the lookup filters them out).
**Pointers:** `crates/adapters/src/dynamo/mod.rs:116-169` (`update_user` status-write path, which backs `delete_user`) · `crates/adapters/src/postgres/mod.rs:28` and `crates/adapters/src/sqlite/mod.rs:28` (unique index → partial) · `crates/adapters/src/postgres/mod.rs:178` and `crates/adapters/src/sqlite/mod.rs:195` (`get_user_by_external_id` filter) · SQL migration steps (`DROP INDEX idx_users_external_id_provider` + partial recreate)

## Steps

- [ ] DynamoDB: when an `update_user` patch sets `status = Deleted`, make the write a `TransactWriteItems` of the versioned status write (from task 08) plus a `Delete` of the `EXT#<provider>#<external_id>` / `UNIQUE` guard item; keep the non-delete path a plain versioned write.
- [ ] Postgres/SQLite: replace the full unique index with `CREATE UNIQUE INDEX … ON users (external_id, provider) WHERE status != 'deleted'` in the inline DDL, and add an explicit migration (`DROP INDEX idx_users_external_id_provider` + partial recreate) for existing tables.
- [ ] Postgres/SQLite: add `AND status != 'deleted'` to `get_user_by_external_id` so a deleted row is never returned to the exchange flow.
- [ ] Confirm `MockRepository` (from task 03) already excludes deleted users from external-id lookup and allows re-create after delete; align it if not.
- [ ] Update `08-persistence.md` (§DynamoDB guard-delete, §PostgreSQL/SQLite partial index + deleted-exclusion), and `schemas/dynamodb/table-design.json` if the delete access pattern is documented.

## Definition of done

- [ ] On each of DynamoDB, Postgres, and SQLite: create a user, delete it, then `create_user` for the same `(provider, external_id)` succeeds as a new user (fresh `id`, empty claims), and `get_user_by_external_id` returns nothing for the deleted identity.
- [ ] Negative-space test: a non-deleted duplicate still conflicts (the partial index / guard still enforces uniqueness among live users) — deletion frees the id, it does not disable uniqueness.
- [ ] The DynamoDB delete removes the guard item in the same transaction as the status write (both succeed or neither), verified against Dynamo Local.
- [ ] The SQL partial-index migration is idempotent for fresh and existing databases (inline `IF NOT EXISTS` DDL plus the explicit `DROP`/recreate step).
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: a reviewer runs the delete-then-re-register test on all three backends and observes a fresh user with no carried-over claims or sessions, and no lookup hit for the deleted identity.
