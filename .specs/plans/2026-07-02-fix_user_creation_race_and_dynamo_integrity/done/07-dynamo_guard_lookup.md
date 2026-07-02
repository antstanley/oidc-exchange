# Task 07 — DynamoDB get_user_by_external_id via the guard item

**Plan:** [plan.md](../plan.md) · **Certificate:** [07-dynamo_guard_lookup-certificate.md](07-dynamo_guard_lookup-certificate.md)

**Implements:** [08-persistence.md](../../../service/specs/08-persistence.md) §DynamoDB (guard-based `get_user_by_external_id`; GSI1 user entry retired; provider-prefix sentence rewrite) · [table-design.json](../../../../schemas/dynamodb/table-design.json) (User item drops GSI1 keys; `access_patterns.get_user_by_external_id` rewritten)
**Depends on:** 06
**Produces:** `get_user_by_external_id` on DynamoDB is two strongly-consistent `GetItem`s — the guard item (`EXT#<provider>#<external_id>` / `UNIQUE`) resolves the `user_id`, then the profile is read at `USER#<id>` / `PROFILE` — retiring the User item's GSI1 entry so GSI1 serves only session lookups.
**Pointers:** `crates/adapters/src/dynamo/mod.rs:56-81` (`get_user_by_external_id`; replace the GSI1 `Query`) · `crates/adapters/src/dynamo/schema.rs:22-27` (drop the User item's `GSI1pk`/`GSI1sk`) · `crates/adapters/src/dynamo/schema.rs:399-413` (the `user_item_has_correct_keys` test asserts GSI keys — update it) · `schemas/dynamodb/table-design.json:25-30,47-64` (access pattern + User item schema)

## Steps

- [x] Replace the GSI1 `Query` in `get_user_by_external_id` with a `consistent_read(true)` `GetItem` on the guard key; on a hit, read `user_id` and issue a second `GetItem` on `USER#<user_id>` / `PROFILE`; return `None` when the guard is absent.
- [x] Remove the `GSI1pk`/`GSI1sk` writes for the User item from `user_to_item` in `schema.rs` (sessions keep theirs); update the `user_item_has_correct_keys` unit test accordingly.
- [x] Update `schemas/dynamodb/table-design.json`: drop `GSI1pk`/`GSI1sk` from `item_schemas.User`, and rewrite `access_patterns.get_user_by_external_id` as the two-`GetItem` guard path.
- [x] Rewrite the `08-persistence.md` §DynamoDB access-pattern row, the guard-based-lookup paragraph, the GSI1-retired sentence, and the closing provider-prefix sentence (now the guard `pk` carries the provider prefix).
- [x] Confirm the guard backfill from task 06 has run before this lookup ships (a guard-less existing user would otherwise become invisible); state the precondition in the task.

## Definition of done

- [x] An integration test (Dynamo Local) creates a user via task 06's transactional create, then `get_user_by_external_id` returns that user by resolving the guard then the profile.
- [x] Negative-space test: `get_user_by_external_id` for an identity with no guard item returns `None` (not an error, not an arbitrary user).
- [x] The lookup uses `consistent_read(true)` on the guard `GetItem`, and the User item no longer carries GSI1 attributes (asserted by the updated `schema.rs` test).
- [x] `schemas/dynamodb/table-design.json` and `08-persistence.md` describe the two-`GetItem` guard lookup with no User GSI1 entry.
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: a reviewer runs the guard-lookup Dynamo Local test and observes the user resolved through the guard, and a missing guard yielding `None`.
