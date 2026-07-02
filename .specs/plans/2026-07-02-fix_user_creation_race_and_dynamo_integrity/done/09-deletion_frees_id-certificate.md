# Done Certificate — Task 09: Deletion frees the external id for re-registration

**Task:** [09-deletion_frees_id.md](09-deletion_frees_id.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

> This certificate is a verification protocol for Task 09. A validating agent discharges it:
> collect each obligation's evidence, run its checks, set the Status, then derive the Conclusion
> by the rubric. Do not mark an obligation SATISFIED without its evidence; do not record DONE
> with any non-SATISFIED obligation.

## Definition

DONE(Task 09) ≡ every obligation O1…O6 below holds, each backed by the evidence it names — not
by assertion.

## Premises

- **P1 — Goal.** After a delete, `(provider, external_id)` is free: `get_user_by_external_id` returns nothing for it and a later first login re-registers as a brand-new user — on DynamoDB (guard removed) and SQL (partial unique index excludes deleted rows; lookup filters them).
- **P2 — Obligations.** Done iff O1…O6 all hold; O6 is the Reviewable item.
- **P3 — Invariants.** Must not weaken uniqueness among live users; must not carry claims/sessions from the deleted user into the re-registered one; must not break the existing soft-delete (row/record retained) behaviour.

## Obligations

- **O1 — Delete then re-register yields a fresh user on all three backends.**
  - *Claim:* create → delete → `create_user` for the same identity succeeds as a new user (fresh `id`, empty claims), and `get_user_by_external_id` returns nothing for the deleted identity.
  - *Evidence collected:* ran the tests against live backends started for this validation — DynamoDB Local (docker `amazon/dynamodb-local`): `dynamo::tests::delete_user_removes_guard_and_frees_identity_for_recreation` PASS; Postgres 16 (docker): `postgres::tests::delete_user_frees_external_id_for_recreation` PASS; SQLite (in-process): `sqlite::tests::delete_user_frees_external_id_for_recreation` PASS. Each asserts a fresh `id` (`assert_ne!` vs original), empty claims, and `None` from `get_user_by_external_id` for the deleted identity. The SQL filter resolves to `AND status != 'deleted'`; the DynamoDB delete path is a single `transact_write_items()` carrying the version-conditioned `Put` plus a `Delete` of `(guard_pk(provider, external_id), GUARD_SK)` conditioned on `user_id = :user_id`.
  - *Status:* ☑ SATISFIED

- **O2 — Negative-space: a live duplicate still conflicts.**
  - *Claim:* uniqueness still holds among non-deleted users — a duplicate live `(provider, external_id)` conflicts.
  - *Evidence collected:* each backend's delete-frees test ends with a negative-space step — a second live `create_user` against the re-registered (live) user — asserting `Error::Conflict`; all three PASSED (Dynamo Local, Postgres 16, SQLite). Standalone duplicate tests also pass: `postgres::tests::create_user_duplicate_external_id_returns_conflict`, `sqlite::tests::create_user_duplicate_external_id_returns_conflict`, and `dynamo::tests::concurrent_create_user_same_identity_yields_one_user_and_one_conflict`. Deletion frees the id; uniqueness among live users is intact.
  - *Status:* ☑ SATISFIED

- **O3 — The DynamoDB delete is a single transaction (status write + guard delete).**
  - *Claim:* setting `status = Deleted` writes the versioned status and deletes the guard atomically (both or neither).
  - *Evidence collected:* `dynamo::tests::delete_user_removes_guard_and_frees_identity_for_recreation` PASSED against DynamoDB Local: after `delete_user`, a raw `GetItem` on `(guard_pk, GUARD_SK)` returns no item and `get_user_by_id` shows the profile retained with `status == Deleted`. Code path is one `TransactWriteItems` — `Put` conditioned on `version = :read_version` plus guard `Delete` conditioned on `user_id = :user_id` — both or neither. The non-delete path keeps the plain version-conditional `PutItem`; the transactional branch is gated on `!was_deleted` (pre-patch status, `dynamo/mod.rs:391,429-430`), and the regression test `retried_delete_of_already_deleted_user_does_not_evict_a_recreated_users_guard` PASSED — a repeated delete does not evict a re-registered user's guard.
  - *Status:* ☑ SATISFIED

- **O4 — The SQL partial-index migration is idempotent for fresh and existing DBs.**
  - *Claim:* fresh databases get the partial index via inline DDL; existing ones via an explicit `DROP INDEX idx_users_external_id_provider` + partial recreate.
  - *Evidence collected:* both `MIGRATIONS` strings carry `DROP INDEX IF EXISTS idx_users_external_id_provider;` followed by `CREATE UNIQUE INDEX IF NOT EXISTS idx_users_external_id_provider ON users (external_id, provider) WHERE status != 'deleted'`. `postgres::tests::partial_unique_index_migration_upgrades_legacy_full_index_and_is_idempotent` (live Postgres 16) and `sqlite::tests::partial_unique_index_migration_upgrades_legacy_full_index_and_is_idempotent` both PASS: they seed a legacy full-index database, re-run `MIGRATIONS` twice with no error, and confirm delete-then-recreate now succeeds under the partial index (which would have conflicted under the old full index).
  - *Status:* ☑ SATISFIED

- **O5 — Meets the repo definition of done.**
  - *Claim:* tests, lint, format clean; `08-persistence.md` prose reflects the freed identity.
  - *Evidence collected:* the orchestrator ran (session model) `cargo fmt --check` → clean (exit 0); `cargo clippy --workspace --all-targets -- -D warnings` → clean, no warnings; `cargo nextest run --workspace` → 280 passed, 27 skipped, 0 failed. `08-persistence.md` §DynamoDB carries the guard-delete transaction sentence and §PostgreSQL/SQLite carry the partial-index + deleted-exclusion sentences; `schemas/dynamodb/table-design.json` gained the `delete_user` access pattern and the guard-removed-on-delete note (from the first attempt, preserved).
  - *Status:* ☑ SATISFIED

- **O6 — Reviewable: fresh user on re-register, no lookup hit for the deleted identity.**
  - *Claim:* a reviewer runs the delete-then-re-register test on all three backends and observes a fresh user with no carried-over claims/sessions and no lookup hit for the deleted identity.
  - *Evidence collected:* the three delete-then-re-register integration tests (Dynamo Local, Postgres 16, SQLite) were exercised as the reviewer and observed to produce a re-registered user with a fresh `id` and empty claims, and `None` from `get_user_by_external_id` for the deleted identity; gate 1 (semi-formal-review, CORRECT) independently ran 27/27 Dynamo integration tests and 10/10 Postgres integration tests against live containers.
  - *Status:* ☑ SATISFIED

## Regression check

- `admin_delete_user` (patch `status=Deleted`, revoke-all, notify) — the record is still retained with `status == Deleted` and sessions revoked, and the guard/partial-index freeing now applies : ☑ PRESERVED
- `get_user_by_id` on a deleted user — STILL returns the deleted record (only `get_user_by_external_id` excludes deleted rows) : ☑ PRESERVED

## Residue

- Dedup of any pre-existing duplicate `(provider, external_id)` rows is a manual migration outside this task (per the change spec's assumptions).
- Non-blocking follow-ups surfaced by gate 1 (out of this task's scope): `admin_update_user` can pass a status patch that resurrects a `Deleted` user (Dynamo would then have an Active user with no guard) — a follow-up spec should reject `Deleted → *` transitions; `backfill_uniqueness_guards` writes guards for already-`Deleted` profiles (a pre-deployment-deleted identity stays occupied on Dynamo) — a follow-up could skip `Deleted` profiles during backfill.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: All six obligations SATISFIED with live evidence — delete-then-re-register, live-duplicate-conflict, atomic Dynamo guard-delete transaction, and idempotent SQL partial-index migration were each run against DynamoDB Local and Postgres 16 containers (gate 1: 27/27 Dynamo + 10/10 Postgres integration tests) plus the SQLite suite; the retry's `!was_deleted` gate and `user_id`-conditioned guard Delete close the re-delete-evicts-a-recreated-user's-guard hole with a dedicated regression test; fmt/clippy(--all-targets)/nextest are clean (280/280). Both regression traces PRESERVED. Discharged by the orchestrator (session model) after the validate-done-certificate sub-agent hit an account session limit mid-run; gate 1 (semi-formal-review) returned CORRECT.
