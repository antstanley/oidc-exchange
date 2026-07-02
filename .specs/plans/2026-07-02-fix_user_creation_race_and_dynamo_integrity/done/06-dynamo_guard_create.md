# Task 06 — DynamoDB transactional create_user with uniqueness guard

**Plan:** [plan.md](../plan.md) · **Certificate:** [06-dynamo_guard_create-certificate.md](06-dynamo_guard_create-certificate.md)

**Implements:** [08-persistence.md](../../../service/specs/08-persistence.md) §DynamoDB (uniqueness-guard item; transactional `create_user`) · [table-design.json](../../../../schemas/dynamodb/table-design.json) (`item_schemas.UserUniquenessGuard`)
**Depends on:** 01, 02
**Produces:** `create_user` on DynamoDB is a `TransactWriteItems` of the user item plus an `EXT#<provider>#<external_id>` / `UNIQUE` guard item carrying `user_id`, each conditioned on `attribute_not_exists(pk)`; a cancelled transaction (guard already present) surfaces as `Error::Conflict`, making `(provider, external_id)` unique at write time.
**Pointers:** `crates/adapters/src/dynamo/mod.rs:84-113` (`create_user`; replace `put_item` with `transact_write_items`) · `crates/adapters/src/dynamo/schema.rs` (add a `guard_to_item` / guard-key helper) · `schemas/dynamodb/table-design.json:47-83` (`item_schemas`)

## Steps

- [x] Add a schema helper building the guard item — `pk = EXT#<provider>#<external_id>`, `sk = UNIQUE`, attribute `user_id: S` — beside `user_to_item` in `schema.rs`.
- [x] Replace the `put_item` in `create_user` with `transact_write_items` of two `Put`s (user item + guard item), each with `condition_expression("attribute_not_exists(pk)")`.
- [x] Map a `TransactionCanceledException` whose cancellation reasons include `ConditionalCheckFailed` to `Error::Conflict { detail }`; map other transaction/SDK errors to `StoreError`.
- [x] Add a guard-backfill step (one-off script or documented lazy-write) that writes a guard item for each existing user, so the invariant holds for pre-existing rows before task 07's lookup ships; note the ordering constraint in the task.
- [x] Add the `UserUniquenessGuard` entry to `schemas/dynamodb/table-design.json` `item_schemas` and update the `08-persistence.md` §DynamoDB item table + transactional-create prose.

## Definition of done

- [x] An integration test (Dynamo Local) creates a user and asserts both the profile item and the `EXT#…` / `UNIQUE` guard item exist with the correct `user_id`.
- [x] A test issuing two `create_user` calls for the same `(provider, external_id)` returns one `User` and one `Error::Conflict` (the transaction cancels on the guard's condition).
- [x] Negative-space test: a `TransactWriteItems` failure that is not a conditional-check cancellation maps to `StoreError`, not `Conflict`.
- [x] `schemas/dynamodb/table-design.json` documents the `UserUniquenessGuard` item and the `08-persistence.md` DynamoDB item table lists the guard row.
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: a reviewer runs the concurrent-create Dynamo Local test and observes exactly one user and one `Conflict`, plus the persisted guard item.

## Open questions

- Whether the guard backfill ships as a committed one-off binary/script or as a lazy write on first lookup miss — the change spec permits either; the choice gates task 07.
