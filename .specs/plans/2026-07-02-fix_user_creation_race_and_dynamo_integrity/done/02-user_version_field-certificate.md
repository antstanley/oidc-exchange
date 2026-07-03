# Done Certificate — Task 02: Store-managed `version` field on User

**Task:** [02-user_version_field.md](02-user_version_field.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

> This certificate is a verification protocol for Task 02. A validating agent discharges it:
> collect each obligation's evidence, run its checks, set the Status, then derive the Conclusion
> by the rubric. Do not mark an obligation SATISFIED without its evidence; do not record DONE
> with any non-SATISFIED obligation.

## Definition

DONE(Task 02) ≡ every obligation O1…O6 below holds, each backed by the evidence it names — not
by assertion.

## Premises

- **P1 — Goal.** `User` carries a store-managed integer `version` that `create_user` writes as `1` and every backend persists and round-trips, with a missing value reading as the migration default `1`.
- **P2 — Obligations.** Done iff O1…O6 all hold; O6 is the Reviewable item.
- **P3 — Invariants.** Must not add `version` to `NewUser` or `UserPatch`; must not break the existing user round-trip tests in `schema.rs`, `postgres`, `sqlite`, or the mock, nor change existing non-`version` fields.

## Obligations

- **O1 — create_user returns version == 1 and it round-trips per backend.**
  - *Claim:* on DynamoDB, Postgres, and SQLite, `create_user` yields `version == 1` and a re-read returns `1`.
  - *Evidence to collect:* run the per-backend create/round-trip tests (`schema.rs` user round-trip; `sqlite` `sqlite_user_crud`; the Postgres/Dynamo Local integration create test) — expect `version == 1` after create and after re-read.
  - *Checks:* resolve `version` reads to `item_to_user`/`row_to_user` in each adapter, not a shadowed local.
  - *Discharged (round 2, 2026-07-02):* DynamoDB — `dynamo::tests::dynamo_repository_crud` run against live DynamoDB Local (docker) → PASS (`created.version == INITIAL_USER_VERSION`, `fetched.version == created.version`); `dynamo::schema::tests::user_round_trip` + `user_round_trip_no_optional_fields` → PASS. SQLite — `sqlite::tests::sqlite_user_crud` and `create_pool_runs_migrations_and_round_trips_version` → PASS. Postgres — the round-1 fixture defect (multi-command `MIGRATIONS` through `sqlx::query`, error 42601) is fixed: `create_test_repo` now applies `sqlx::raw_sql(MIGRATIONS)` on a dedicated connection under a session advisory lock; `postgres::tests::create_user_round_trips_initial_version` run against live Postgres 16 (docker) → PASS (`created.version == 1`, re-read `== 1`). Checks: `version` resolves to `get_version_or_default` in `item_to_user` (dynamo/schema.rs), `row.try_get::<i64,_>("version")` in postgres `row_to_user`, `row.get::<i64,_>("version")` in sqlite `row_to_user` — no shadowing.
  - *Status:* ☒ SATISFIED

- **O2 — A pre-existing record with no version reads as 1.**
  - *Claim:* a DynamoDB item lacking the `version` attribute and a SQL row via `DEFAULT 1` both yield `version == 1`.
  - *Evidence to collect:* run a `schema.rs` test that builds an item without `version` and asserts `item_to_user` returns `1`; inspect the SQL `CREATE TABLE` / `ALTER TABLE` DDL for `version BIGINT NOT NULL DEFAULT 1`. Expect PASS / DDL present.
  - *Discharged (round 2, 2026-07-02):* `dynamo::schema::tests::item_to_user_missing_version_defaults_to_initial_version` → PASS (plus `item_to_user_non_numeric_version_returns_error` → PASS). Postgres `MIGRATIONS` carries `version BIGINT NOT NULL DEFAULT 1` in `CREATE TABLE` plus `ALTER TABLE users ADD COLUMN IF NOT EXISTS version BIGINT NOT NULL DEFAULT 1`; `postgres::tests::legacy_row_without_version_column_defaults_to_initial_version` (drops the column, inserts a legacy row, re-runs `MIGRATIONS`, reads back `1`) run against live Postgres 16 (docker) → PASS. SQLite DDL carries `version INTEGER NOT NULL DEFAULT 1` plus `ensure_version_column` (pragma-guarded ALTER); `sqlite::tests::legacy_row_without_version_column_defaults_to_initial_version` and `ensure_version_column_is_idempotent` → PASS.
  - *Status:* ☒ SATISFIED

- **O3 — Both schemas and the domain page list version.**
  - *Claim:* `version` is in `properties` and `required` of `User` in the service `canonical-types.schema.json` and `datamodel.schema.json`, and the `01-domain-model.md` struct listing matches.
  - *Evidence to collect:* read `$defs.User` in `.specs/service/specs/canonical-types.schema.json` and `definitions.User` in `schemas/datamodel.schema.json`; confirm `version` in both `properties` and `required`. Read the `01-domain-model.md` User struct — confirm the `version: u64` line and store-managed prose.
  - *Discharged (round 2, 2026-07-02):* both schemas list `version` (`"type": "integer", "minimum": 1`) in `properties` and include it in `required`. `01-domain-model.md` struct listing carries `version: u64` with the store-managed comment, plus the store-managed prose paragraph (never caller-supplied, not in `NewUser`/`UserPatch`), the Deleted freed-identity bullet, and the non-deleted-uniqueness Assumptions bullet; `08-persistence.md` carries the PostgreSQL and SQLite `version`-column sentences.
  - *Status:* ☒ SATISFIED

- **O4 — Every User constructor sets version; the workspace builds.**
  - *Claim:* no `User { … }` literal omits `version`.
  - *Evidence to collect:* `grep -rn "User {" crates bindings` and confirm each construction sets `version`; run `cargo build --workspace` — expect no missing-field error.
  - *Discharged (round 2, 2026-07-02):* grep over `crates` + `bindings` finds 10 `User { … }` literal constructions (dynamo/mod.rs create_user, dynamo/schema.rs `item_to_user` + 2 test builders, postgres `row_to_user`, sqlite `row_to_user` + create_user, webhook test builder, core claims test builders x2, test-utils mock create_user, core tests/claims.rs) — every one sets `version`. `cargo clippy --workspace -- -D warnings` (type-checks all crates incl. bindings) → clean; `cargo build --workspace --exclude oidc-exchange-python` → clean. `cargo build --workspace` fails only at the `oidc-exchange-python` cdylib **link** step (pre-existing macOS pyo3 extension-module linking, needs maturin's `-undefined dynamic_lookup`; unrelated to this task) — no missing-field error anywhere.
  - *Status:* ☒ SATISFIED

- **O5 — Meets the repo definition of done.**
  - *Claim:* tests, lint, format clean; schema + prose updated together.
  - *Evidence to collect:* run `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` — expect clean.
  - *Discharged (round 2, 2026-07-02):* `cargo fmt --all --check` → clean. `cargo clippy --workspace -- -D warnings` → clean. `cargo nextest run --workspace` → 259 passed, 0 failed (12 skipped = `#[ignore]` integration tests). Named constant `INITIAL_USER_VERSION` used throughout instead of a bare `1`. The round-1 caveat is resolved: all four `#[ignore]`d version-related integration tests (2 Postgres, 1 Dynamo CRUD, 1 SQLite legacy) now PASS against live backends (docker Postgres 16 + DynamoDB Local).
  - *Status:* ☒ SATISFIED

- **O6 — Reviewable: round-trip and schema validation show version == 1 and required.**
  - *Claim:* a reviewer runs the per-backend round-trip tests and schema validation and observes `version == 1` on create and `version` required.
  - *Evidence to collect:* run the round-trip tests for the three backends; validate a `User` missing `version` against the updated schemas — expect INVALID (required), and a create-produced `User` — expect valid with `version: 1`.
  - *Discharged (round 2, 2026-07-02):* schema validation re-exercised with ajv (draft 2020-12) against both `schemas/datamodel.schema.json#/definitions/User` and the service `canonical-types.schema.json#/$defs/User` (repo-root `canonical-types.schema.json` registered for the `../../` refs): a create-produced `User` with `version: 1` → VALID in both; the same object minus `version` → INVALID in both (`must have required property 'version'`). Round-trip tests: DynamoDB (live Local) → PASS, SQLite → PASS, Postgres (live 16) → PASS — a reviewer can now observe all three backends round-trip `version == 1`.
  - *Status:* ☒ SATISFIED

## Regression check

- `item_to_user`/`row_to_user` are called by `get_user_by_id`, `get_user_by_external_id`, `list_users`; trace `get_user_by_id` after the field addition → expect the returned `User` populated with all prior fields plus `version` : ☒ PRESERVED — Dynamo `dynamo_repository_crud` (live) fetched the created user with all prior fields plus `version`; sqlite `sqlite_user_crud` asserts the same; postgres `create_user_round_trips_initial_version` (live) fetches via `get_user_by_id` → all prior fields plus `version == 1` (queries are `SELECT *`, so `row_to_user`'s `try_get("version")` sees the column); missing-attribute default covered by the O2 tests.
- `MockRepository::create_user` callers in `crates/core/tests/*` → expect they still compile and return a `User` : ☒ PRESERVED — `cargo nextest run --workspace` green (259/259), including `core/tests/{exchange,claims,user_admin}.rs`.

## Residue

- The Deleted freed-identity prose added to `01-domain-model.md` here is realized by task 09; a validator should verify only that the prose exists, not the deletion behaviour.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O6 all SATISFIED with evidence in hand and both regression traces PRESERVED — the round-1 blocker (Postgres test fixture ran multi-command `MIGRATIONS` through `sqlx::query`, error 42601) was fixed by switching the fixture to `sqlx::raw_sql(MIGRATIONS)` under a session advisory lock, and on re-validation all four `#[ignore]`d integration tests pass against live Postgres 16 and DynamoDB Local (docker), the default suite is green (259/259), fmt/clippy clean, and ajv confirms both schemas require `version`. Invariants held: `NewUser`/`UserPatch` unchanged, no existing test broken. Environment note (unchanged from round 1, not a defect): `cargo build --workspace` fails only at the pre-existing `oidc-exchange-python` pyo3 cdylib link step on macOS (needs maturin's `-undefined dynamic_lookup`); `--exclude oidc-exchange-python` builds clean and clippy type-checks every crate.
