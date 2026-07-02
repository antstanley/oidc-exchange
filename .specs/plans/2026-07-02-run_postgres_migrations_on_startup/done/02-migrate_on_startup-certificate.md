# Done Certificate — Task 02: migrate on startup

**Task:** [02-migrate_on_startup.md](02-migrate_on_startup.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

> This certificate is a verification protocol for Task 02. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 02) ≡ every obligation O1…O6 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** `create_pool(url, max_connections, run_migrations)` runs `MIGRATIONS` via sqlx's raw path on a fresh Postgres database when `run_migrations` is true and only connects when false, with a gated integration test proving a fresh database serves `create_user` after `create_pool` alone.
- **P2 — Obligations.** The task is done iff O1…O6 all hold. One Oi per definition-of-done item, in DoD order; O6 is the `Reviewable:` item.
- **P3 — Invariants.** Must not break the existing `create_pool` connection behaviour, the two `bootstrap.rs` Postgres call sites (which must keep compiling), or the existing SQLite `create_pool` path it mirrors.

## Obligations

- **O1 — `create_pool(url, n, true)` migrates a fresh DB; `create_pool(url, n, false)` only connects.**
  - *Claim:* with `run_migrations` true, `sqlx::raw_sql(MIGRATIONS).execute(&pool)` runs so `users`/`sessions` and their indexes exist; with false, no DDL runs.
  - *Evidence to collect:* read `crates/adapters/src/postgres/mod.rs` `create_pool` (was lines 60-68); confirm the signature is `create_pool(url, max_connections, run_migrations: bool) -> Result<PgPool, sqlx::Error>`, the migration is `sqlx::raw_sql(MIGRATIONS)` guarded by `if run_migrations`, and the false path returns the pool unmigrated.
  - *Checks:* confirm `sqlx::raw_sql` (simple-query path) is used, not `sqlx::query`/prepared statements — a prepared statement cannot run the multi-statement `MIGRATIONS` block on Postgres. Resolve `MIGRATIONS` to `postgres::MIGRATIONS` (lines 14-42), not the SQLite constant.
  - *Status:* SATISFIED — `crates/adapters/src/postgres/mod.rs:125-140`: signature is `create_pool(url: &str, max_connections: u32, run_migrations: bool) -> std::result::Result<PgPool, sqlx::Error>`; migration is `sqlx::raw_sql(MIGRATIONS).execute(&pool).await?` (line 136, simple-query path, not `sqlx::query`) guarded by `if run_migrations` (line 135); false path falls through to `Ok(pool)` with no DDL. `MIGRATIONS` resolves to the module-local `postgres::MIGRATIONS` constant at lines 15-54 of the same file (no import, no SQLite shadow). Live-DB confirmation: after `create_pool(url, 2, true)` on a fresh schema, `pg_tables`/`pg_indexes` show `users`, `sessions`, `idx_users_external_id_provider`, `idx_sessions_user_id`; after `create_pool(url, 2, false)` on a second fresh schema, both catalogs show nothing.

- **O2 — Gated integration test proves fresh-DB service and the false-path emptiness.**
  - *Claim:* with `DATABASE_URL` set, a test shows `create_user` succeeds after `create_pool(url, n, true)` alone, and `create_pool(url, n, false)` leaves a fresh schema with no tables; with `DATABASE_URL` unset the test skips.
  - *Evidence to collect:* locate the gated Postgres test; run `cargo nextest run --workspace` with `DATABASE_URL` set to a scratch Postgres — expect the test to PASS; run again with `DATABASE_URL` unset — expect the test to be skipped, not failed.
  - *Checks:* confirm the skip is a genuine early-return/skip on the missing env var, not a silently-passing empty test.
  - *Status:* SATISFIED — test `postgres::tests::create_pool_migrates_on_startup_and_run_migrations_false_stays_bare` (`crates/adapters/src/postgres/mod.rs`). With `DATABASE_URL` pointed at a scratch Postgres 16 (Docker, port 55432): targeted run PASS in 0.281s (real body — `--no-capture` shows no skip message) and `cargo nextest run --workspace` → 282 passed, 0 failed. With `DATABASE_URL` unset: `cargo nextest run --workspace` → 282 passed (test completes in 0.017s via the guard). Skip check: the gate is `let Ok(base_url) = std::env::var("DATABASE_URL") else { eprintln!("skipping …: DATABASE_URL is not set"); return; }` — a genuine early return on the missing env var; the body below it creates users and probes catalogs, so it is not an empty test.

- **O3 — Negative-space: `run_migrations = false` leaves no tables.**
  - *Claim:* after `create_pool(url, n, false)` on a fresh schema, `to_regclass('users')` and `to_regclass('sessions')` are null.
  - *Evidence to collect:* in the gated test, confirm the false-path assertion probes for the tables' absence (e.g. `to_regclass` is null or a `SELECT` against `users` errors with undefined_table); run it against a fresh schema with `DATABASE_URL` set — expect PASS.
  - *Status:* SATISFIED — the test's second half resets a separate fresh schema (`oidc_adapter_test_migrate_on_startup_false`, `DROP SCHEMA … CASCADE; CREATE SCHEMA`), calls `create_pool(&bare_url, 2, false)`, then `SELECT to_regclass('users'), to_regclass('sessions')` and asserts both are `None` (`users_reg.is_none()` / `sessions_reg.is_none()`), with the pool's `search_path` pinned to the bare schema so `public` cannot leak in. Ran against the scratch Postgres — PASS; direct `pg_tables`/`pg_indexes` inspection of that schema afterwards shows zero tables and zero indexes.

- **O4 — 08-persistence.md documents migrate-on-startup and connect-only.**
  - *Claim:* `08-persistence.md` §PostgreSQL states `create_pool` runs the idempotent migrations before returning unless `run_migrations = false`, in which case it only connects; the page `**Date:**` is bumped.
  - *Evidence to collect:* read `.specs/service/specs/08-persistence.md` §PostgreSQL (around lines 34-38); confirm the migrate-on-startup and `run_migrations = false` prose is present and the header `**Date:**` differs from `2026-06-24`.
  - *Status:* SATISFIED — `.specs/service/specs/08-persistence.md` §PostgreSQL now reads "`create_pool(url, max_connections, run_migrations)` builds the connection pool and, unless `run_migrations` is `false`, executes the adapter's idempotent migrations … run via sqlx's raw simple-query path … before returning — like SQLite" and "With `run_migrations = false`, `create_pool` only connects, leaving DDL to an out-of-band process". Header `**Date:**` is `2026-07-02` (bumped from `2026-06-24`).

- **O5 — Meets the repo definition of done.**
  - *Claim:* tests pass, format and lint clean, ≥2 assertions per touched function, limits named.
  - *Evidence to collect:* from `.specs/development-guidelines.md` §Definition of done, run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, and `cargo nextest run --workspace` — expect all clean; confirm the new/touched test functions carry ≥2 meaningful assertions.
  - *Status:* SATISFIED — `cargo fmt --check` exit 0; `cargo clippy --workspace -- -D warnings` clean; `cargo nextest run --workspace` 282 passed / 0 failed (both with and without `DATABASE_URL`). The new test carries 4 meaningful assertions (2 `assert_eq!` on the round-tripped user, 2 `assert!(…is_none())` on table absence) plus expect-messages on every fallible step. No new numeric limits introduced beyond documented test fixtures; `create_pool`'s behaviour change is doc-commented.

- **O6 — Reviewable: gated test shows create_user after migrate and no tables after connect-only (Reviewable).**
  - *Claim:* a reviewer runs the gated test against a scratch Postgres and observes `create_user` succeeding after `create_pool(..., true)` alone and the tables absent after `create_pool(..., false)`.
  - *Evidence to collect:* `DATABASE_URL=<scratch> cargo nextest run -p oidc-exchange-adapters <test name>`; observe the create-user-after-migrate assertion and the no-tables-after-false assertion both passing.
  - *Status:* SATISFIED — exercised as the reviewer would: scratch Postgres 16 in Docker, `DATABASE_URL=postgres://postgres:cert02@localhost:55432/cert02 cargo nextest run -p oidc-exchange-adapters create_pool_migrates_on_startup` → PASS (0.281s). Post-run inspection via psql: the migrated schema holds the created user row (`google|pg_migrate_on_startup_test`), both tables, and both indexes; the connect-only schema holds no tables at all — both halves of the contract observed on a live database, not inferred.

## Regression check

- `crates/server/src/bootstrap.rs:185` and `:244` call `create_pool`. Trace: after the signature gains `run_migrations: bool`, both call sites compile (task 02 passes a placeholder `true`) and the server still builds : PRESERVED — both call sites (now bootstrap.rs:370-375 and :433-438) pass `true` with a `TODO(task 04)` comment; the whole workspace builds clean (clippy `-D warnings`) and all 282 workspace tests pass. Behaviour at the call sites is unchanged-or-better: previously the pool was returned unmigrated; now startup migrates, which is the task's goal.
- The SQLite `create_pool` (`crates/adapters/src/sqlite/mod.rs:54-79`) is untouched and its existing tests still pass : PRESERVED — `jj diff` shows no change to `crates/adapters/src/sqlite/`; `cargo nextest run -p oidc-exchange-adapters -E 'test(/sqlite/)' --run-ignored=all` → 13/13 passed. Additionally the 10 pre-existing `#[ignore]`d Postgres tests — whose helpers `create_test_repo`/`create_isolated_schema_repo` were edited to pass `run_migrations = false` — all pass against the scratch database (`--run-ignored=all` with `POSTGRES_TEST_URL` set → 11/11 including the new test).

## Residue

- The denied-DDL degrade branch (`SQLSTATE 42501`) is Task 03, not an obligation here — task 02's migration failure path returns the raw `sqlx::Error`.
- The placeholder `true` passed at the bootstrap call sites is replaced by the config-driven value in Task 04.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O6 all SATISFIED with direct evidence — the gated test was exercised both unset (282/282 workspace green, guard early-returns) and against a live scratch Postgres 16 (PASS, catalogs confirm tables/indexes present after `true` and absent after `false`) — and both regression traces are PRESERVED (bootstrap call sites compile and pass `true`; SQLite untouched, 13/13; all 10 pre-existing ignored Postgres tests still pass with the modified helpers).
