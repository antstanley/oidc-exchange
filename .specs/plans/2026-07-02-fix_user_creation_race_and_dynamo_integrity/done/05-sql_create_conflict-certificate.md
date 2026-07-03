# Done Certificate — Task 05: SQL create_user conflict mapping

**Task:** [05-sql_create_conflict.md](05-sql_create_conflict.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

> This certificate is a verification protocol for Task 05. A validating agent discharges it:
> collect each obligation's evidence, run its checks, set the Status, then derive the Conclusion
> by the rubric. Do not mark an obligation SATISFIED without its evidence; do not record DONE
> with any non-SATISFIED obligation.

## Definition

DONE(Task 05) ≡ every obligation O1…O5 below holds, each backed by the evidence it names — not
by assertion.

## Premises

- **P1 — Goal.** A `create_user` insert that violates the `(external_id, provider)` unique index on Postgres (`23505`) or SQLite (`2067`) returns `Error::Conflict`, not `Error::StoreError`.
- **P2 — Obligations.** Done iff O1…O5 all hold; O5 is the Reviewable item.
- **P3 — Invariants.** Must not change the success path of `create_user` (id minting, returned `User`), nor reclassify non-unique insert failures.

## Obligations

- **O1 — A duplicate insert returns Conflict on both SQL backends.**
  - *Claim:* inserting a second user with the same `(external_id, provider)` returns `Error::Conflict` on Postgres and SQLite.
  - *Evidence to collect:* run the duplicate-insert integration test for each backend — expect the second `create_user` yields `Err(Error::Conflict { .. })`.
  - *Checks:* resolve the error classification to the per-adapter unique-violation helper, and confirm it reads the driver's structured code (`23505` / `2067`), not a message substring.
  - *Status:* ☑ SATISFIED — `postgres::tests::create_user_duplicate_external_id_returns_conflict` PASS against a live Postgres 16 (temporary Docker container), and `sqlite::tests::create_user_duplicate_external_id_returns_conflict` PASS in-memory; both second inserts yielded `Err(Error::Conflict { .. })`. Classification resolves to `is_unique_violation` in each adapter (`crates/adapters/src/postgres/mod.rs:72`, called at `mod.rs:240`; `crates/adapters/src/sqlite/mod.rs:122`, called at `mod.rs:284`), each reading the structured driver code.

- **O2 — Negative-space: a non-unique failure stays StoreError.**
  - *Claim:* a NOT NULL / type / other insert failure maps to `Error::StoreError`, not `Conflict`.
  - *Evidence to collect:* run the test injecting a non-unique-violation insert error — expect `StoreError`.
  - *Status:* ☑ SATISFIED — `create_user_non_unique_failure_maps_to_store_error` PASS on both backends (table dropped out from under the insert: Postgres `42P01` undefined_table, SQLite "no such table"); each returned `Error::StoreError`, not `Conflict`. The classifier tests additionally probe a NOT NULL violation and assert it is not classified as unique.

- **O3 — The classifier asserts on the structured code.**
  - *Claim:* the unique-violation detection uses the driver's error code, not string matching.
  - *Evidence to collect:* read the per-adapter classifier; confirm it inspects the `sqlx` DB error code/kind (`.code()` / constraint kind), and is called from `create_user`.
  - *Status:* ☑ SATISFIED — both classifiers use `err.as_database_error().and_then(|db_err| db_err.code())` compared against a named constant (`PG_UNIQUE_VIOLATION_CODE = "23505"`, `SQLITE_UNIQUE_VIOLATION_CODE = "2067"`); no message substring matching. `is_unique_violation_reads_structured_code_not_message` PASS on both backends: a genuine unique violation classifies true with the expected code, a NOT NULL violation classifies false. Each `create_user` calls `is_unique_violation` by name.

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests, lint, format clean; `08-persistence.md` prose updated.
  - *Evidence to collect:* run `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` — expect clean; confirm the PostgreSQL/SQLite unique-violation → `Conflict` sentence is present.
  - *Status:* ☑ SATISFIED — `cargo fmt --all --check` clean; `cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo nextest run --workspace` 270 passed / 0 failed (15 skipped = `#[ignore]`d live-service tests). `08-persistence.md` now documents the `23505` → `Conflict` mapping in §PostgreSQL and the `2067` (`SQLITE_CONSTRAINT_UNIQUE`) mapping in §SQLite. Codes are named constants, not magic literals.

- **O5 — Reviewable: duplicate insert yields Conflict on both backends.**
  - *Claim:* a reviewer runs the duplicate-insert tests on Postgres and SQLite and observes `Conflict` on the second insert.
  - *Evidence to collect:* run both tests; confirm the SQLite in-memory harness ran `MIGRATIONS` (so the unique index exists) before the duplicate insert.
  - *Status:* ☑ SATISFIED — validator ran both duplicate-insert tests and observed `Conflict` on the second insert (SQLite in-memory; Postgres 16 in a temporary Docker container via `POSTGRES_TEST_URL` default). The SQLite harness `create_test_repo` splits `MIGRATIONS` on `;` and executes every statement, including `CREATE UNIQUE INDEX idx_users_external_id_provider` (`sqlite/mod.rs:31`), before the inserts; the Conflict result itself proves the index was present.

## Regression check

- `create_user` success path callers (`admin_create_user`, exchange JIT create) — trace a first insert → expect the returned `User` unchanged (id, fields) : ☑ PRESERVED — the success path is untouched (only the `map_err` closure changed); `postgres::tests::create_user_round_trips_initial_version` PASS against live Postgres, `sqlite_user_crud` PASS, and the full workspace suite (270 tests, exchange/admin flows included) is green.
- Existing `sqlite_user_crud` test — expect still green after the error-mapping change : ☑ PRESERVED — PASS in the targeted run and in the workspace run.

## Residue

- Whether the SQLite test harness path (`:memory:`, split-statement migrations) creates the unique index is called out as an open question in the task; a validator should confirm it before trusting the duplicate-insert test. — RESOLVED: `create_test_repo` executes every `MIGRATIONS` statement (split on `;`), including the `CREATE UNIQUE INDEX` at `sqlite/mod.rs:31`, and the duplicate insert did fail with the unique-violation code `2067`, which is only possible if the index exists.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: All five obligations discharged with direct evidence: duplicate `(external_id, provider)` inserts return `Error::Conflict` on both Postgres (live, `23505`) and SQLite (in-memory, `2067`); non-unique insert failures still map to `StoreError`; the per-adapter `is_unique_violation` classifiers read the driver's structured code via named constants and are called from `create_user`; fmt/clippy/nextest (270/270) are clean and `08-persistence.md` documents the mapping; both regression checks PRESERVED and the SQLite-index residue resolved.
