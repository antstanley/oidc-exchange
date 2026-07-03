# Done Certificate — Task 06: DynamoDB transactional create_user with uniqueness guard

**Task:** [06-dynamo_guard_create.md](06-dynamo_guard_create.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

> This certificate is a verification protocol for Task 06. A validating agent discharges it:
> collect each obligation's evidence, run its checks, set the Status, then derive the Conclusion
> by the rubric. Do not mark an obligation SATISFIED without its evidence; do not record DONE
> with any non-SATISFIED obligation.

## Definition

DONE(Task 06) ≡ every obligation O1…O6 below holds, each backed by the evidence it names — not
by assertion.

## Premises

- **P1 — Goal.** `create_user` on DynamoDB is a `TransactWriteItems` of the user item plus an `EXT#<provider>#<external_id>` / `UNIQUE` guard item carrying `user_id`, each conditioned on `attribute_not_exists(pk)`; a cancelled transaction surfaces as `Conflict`.
- **P2 — Obligations.** Done iff O1…O6 all hold; O6 is the Reviewable item.
- **P3 — Invariants.** Must not change the returned `User` shape (including `version` from task 02); must not break the existing `dynamo_repository_crud` integration test's create step.

## Obligations

- **O1 — Create writes both the profile and the guard item.**
  - *Claim:* after `create_user`, the profile item (`USER#<id>`/`PROFILE`) and the guard item (`EXT#<provider>#<external_id>`/`UNIQUE`, `user_id` = the new id) both exist.
  - *Evidence to collect:* run the Dynamo Local integration test that creates a user then `GetItem`s both keys — expect both present with the correct `user_id`.
  - *Checks:* resolve the guard-item builder to the new `schema.rs` helper; confirm both `Put`s carry `condition_expression("attribute_not_exists(pk)")`.
  - *Status:* ☑ SATISFIED — ran `dynamo::tests::create_user_writes_profile_and_guard_items` against DynamoDB Local (docker, port 8000): PASS. The test `GetItem`s `USER#<id>`/`PROFILE` and `EXT#google#google|guard_write_test`/`UNIQUE` and asserts `user_id` equals the created id. `guard_to_item` at `crates/adapters/src/dynamo/mod.rs:259` resolves to the `schema.rs` helper (`schema.rs:115`, imported at `mod.rs:19` — no shadow). Both `Put`s carry `condition_expression("attribute_not_exists(pk)")` (`mod.rs:264`, `mod.rs:271`).

- **O2 — A duplicate create returns one user and one Conflict.**
  - *Claim:* two `create_user` calls for the same `(provider, external_id)` yield one `Ok(User)` and one `Err(Conflict)`.
  - *Evidence to collect:* run the concurrent/sequential duplicate-create Dynamo Local test — expect exactly one `Conflict` from the cancelled transaction.
  - *Checks:* confirm the `TransactionCanceledException` with a `ConditionalCheckFailed` reason maps to `Error::Conflict`, resolved against the AWS SDK error type.
  - *Status:* ☑ SATISFIED — ran `dynamo::tests::concurrent_create_user_same_identity_yields_one_user_and_one_conflict` (two racing `create_user` calls via `tokio::join!`) against DynamoDB Local: PASS — exactly one `Ok(User)`, exactly one `Err(Error::Conflict)`. Mapping at `mod.rs:282-296` matches `TransactWriteItemsError::TransactionCanceledException` (SDK type imported at `mod.rs:8` from `aws_sdk_dynamodb::operation::transact_write_items`) and scans `cancellation_reasons()` for the named constant `CONDITIONAL_CHECK_FAILED_CODE` = `"ConditionalCheckFailed"` (`mod.rs:24`).

- **O3 — Negative-space: a non-conditional transaction failure stays StoreError.**
  - *Claim:* a `TransactWriteItems` failure not caused by a conditional-check cancellation maps to `StoreError`.
  - *Evidence to collect:* run/read the test or mapping code covering a non-`ConditionalCheckFailed` cancellation or other SDK error — expect `StoreError`.
  - *Status:* ☑ SATISFIED — ran `dynamo::tests::create_user_non_conditional_failure_maps_to_store_error` (create against a nonexistent table → `ResourceNotFoundException`) against DynamoDB Local: PASS — got `Error::StoreError`, not `Conflict`. In the mapping code the `_ => false` arm at `mod.rs:287` routes every non-`TransactionCanceledException`/non-`ConditionalCheckFailed` error to `Self::store_err` (`mod.rs:298`).

- **O4 — The guard item is documented in the design sidecar and prose.**
  - *Claim:* `schemas/dynamodb/table-design.json` has a `UserUniquenessGuard` entry and `08-persistence.md` lists the guard row and transactional-create prose.
  - *Evidence to collect:* read `item_schemas.UserUniquenessGuard` (pk/sk/`user_id`) in the sidecar; read the `08-persistence.md` §DynamoDB item table + transactional-create paragraph.
  - *Status:* ☑ SATISFIED — `schemas/dynamodb/table-design.json:83-90` adds `item_schemas.UserUniquenessGuard` with `pk = EXT#<provider>#<external_id>`, `sk = UNIQUE`, attribute `user_id: S`, and a description of the transactional write. `.specs/service/specs/08-persistence.md` item table gains the `User uniqueness guard` row, and a new paragraph documents the two-`Put` `TransactWriteItems`, the `ConditionalCheckFailed` → `Conflict` mapping, other failures → `StoreError`, and the idempotent `backfill_uniqueness_guards` migration plus its ordering constraint before task 07's lookup switch.

- **O5 — Meets the repo definition of done.**
  - *Claim:* tests, lint, format clean.
  - *Evidence to collect:* run `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` — expect clean.
  - *Status:* ☑ SATISFIED — `cargo fmt --all --check`: clean (exit 0). `cargo clippy --workspace -- -D warnings`: clean (exit 0). `cargo nextest run --workspace`: 270 passed, 19 skipped (the skips are the `#[ignore]` Dynamo/Postgres-Local integration tests, run separately below). The cancellation-reason code is a named constant (`CONDITIONAL_CHECK_FAILED_CODE`), and `GUARD_SK` / `guard_pk` are named in `schema.rs`.

- **O6 — Reviewable: concurrent create yields one user, one Conflict, and a persisted guard.**
  - *Claim:* a reviewer runs the concurrent-create Dynamo Local test and observes one user, one `Conflict`, and the guard item.
  - *Evidence to collect:* run the test; `GetItem` the guard key and confirm it holds the winning `user_id`.
  - *Status:* ☑ SATISFIED — exercised as reviewer: started DynamoDB Local (docker `amazon/dynamodb-local`, port 8000) and ran `cargo nextest run -p oidc-exchange-adapters --run-ignored all -E 'test(dynamo)'` — 20/20 PASS, including the concurrent-create race test, which asserts exactly one `Ok(User)`, exactly one `Error::Conflict`, and then `GetItem`s the guard key (`EXT#google#google|guard_conflict_test` / `UNIQUE`) confirming `user_id` equals the winner's id. The backfill test (`backfill_writes_guards_for_legacy_users_and_is_idempotent`) also passed: 1 guard written for the legacy user, 0 on re-run, existing guards untouched.

## Regression check

- `create_user` callers (`admin_create_user`, exchange JIT create) — trace a first create → expect an `Ok(User)` with `version == 1` and all fields, unchanged externally : ☑ PRESERVED — `create_user` (`mod.rs:240-303`) still builds `full_user` with `version: INITIAL_USER_VERSION` (`mod.rs:253`) and returns it unchanged on success; the trait signature and returned `User` shape are untouched, only the write became transactional and duplicates now surface `Conflict` (the intended new behavior).
- The existing `dynamo_repository_crud` test's create step → expect still green (or updated to assert the guard) : ☑ PRESERVED — `dynamo::tests::dynamo_repository_crud` PASS against DynamoDB Local in the same run.

## Residue

- The guard backfill for pre-existing users (script vs lazy write) is an open question in the task and a precondition for task 07; not itself an obligation of Task 06 beyond providing the mechanism.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O6 all SATISFIED with direct evidence — fmt/clippy/nextest clean (270 passed), all 20 Dynamo tests (including the four new guard/backfill/conflict tests) run green against a live DynamoDB Local, the docs obligations verified by reading the sidecar and spec diffs, and both regression traces PRESERVED (`dynamo_repository_crud` green; first-create `Ok(User)` with `version == 1` unchanged). The backfill residue is discharged by `backfill_uniqueness_guards` plus its documented ordering constraint for task 07.
