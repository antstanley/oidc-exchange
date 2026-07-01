# Task 02 — migrate on startup

**Plan:** [plan.md](../plan.md) · **Certificate:** [02-migrate_on_startup-certificate.md](02-migrate_on_startup-certificate.md)

**Implements:** [08-persistence.md](../../../service/specs/08-persistence.md) §PostgreSQL (`adapters/postgres`) — `create_pool` runs the idempotent migrations before returning, like SQLite; with `run_migrations = false`, `create_pool` only connects
**Depends on:** —
**Produces:** `create_pool(url, max_connections, run_migrations)` runs `MIGRATIONS` via sqlx's raw (simple-query) path on a fresh Postgres database when `run_migrations` is true, and only connects when false; a gated integration test proves a fresh database serves `create_user` after `create_pool` alone, and that `run_migrations = false` leaves it without tables
**Pointers:** `crates/adapters/src/postgres/mod.rs:60-68` (`create_pool`), `postgres/mod.rs:14-42` (`MIGRATIONS`); mirror `crates/adapters/src/sqlite/mod.rs:54-79`; spec page `.specs/service/specs/08-persistence.md:34-38` (PostgreSQL section)

## Steps

- [ ] Change `create_pool` in `crates/adapters/src/postgres/mod.rs` to take `run_migrations: bool` after `max_connections`, keeping the return type `Result<PgPool, sqlx::Error>`.
- [ ] After `connect`, when `run_migrations` is true, execute the multi-statement `MIGRATIONS` via `sqlx::raw_sql(MIGRATIONS).execute(&pool)` (simple-query protocol — prepared statements cannot run multiple statements on Postgres); return the original `sqlx::Error` on failure. Leave the degrade-on-denied-DDL branch to task 03.
- [ ] When `run_migrations` is false, return the pool without executing any DDL.
- [ ] Add a `run_migrations` argument at the two bootstrap call sites (`crates/server/src/bootstrap.rs:185`, `:244`) sufficient to keep the workspace compiling — pass `true` for now; task 04 replaces it with the config-driven value. (Keeps the tree green without pre-empting 04's wiring.)
- [ ] Add a Postgres integration test gated on a `DATABASE_URL` env var (skip cleanly when unset, so `cargo nextest run --workspace` stays green without a database): assert that after `create_pool(url, n, true)` alone a `PostgresRepository` serves `create_user` and reads it back, and that after `create_pool(url, n, false)` on a fresh schema the `users`/`sessions` tables do not exist.
- [ ] Update `08-persistence.md` §PostgreSQL: `create_pool` builds the pool and, unless `run_migrations = false`, executes the idempotent migrations before returning (like SQLite); with `run_migrations = false`, `create_pool` only connects; bump the page's `**Date:**`.

## Definition of done

- [ ] `create_pool(url, max_connections, true)` against a fresh Postgres database runs `MIGRATIONS` via `sqlx::raw_sql` so the `users` and `sessions` tables (and their indexes) exist afterwards; `create_pool(url, max_connections, false)` connects without creating them.
- [ ] Gated integration test (`DATABASE_URL`) proves a fresh database serves `create_user` after `create_pool(url, n, true)` alone, and that `run_migrations = false` leaves a fresh schema with no tables; the test skips (does not fail) when `DATABASE_URL` is unset.
- [ ] Negative-space coverage: the `run_migrations = false` path is asserted to leave the database without tables (the migration is genuinely conditional, not always-on).
- [ ] `08-persistence.md` §PostgreSQL documents the migrate-on-startup behaviour and the `run_migrations = false` connect-only path; the page `**Date:**` is bumped.
- [ ] Meets the repo definition of done (tests, `cargo fmt`, `cargo clippy --workspace -- -D warnings`, ≥2 assertions per touched function, named-constant limits — see plan.md baseline).
- [ ] Reviewable: a reviewer points `DATABASE_URL` at a scratch Postgres, runs the gated test, and observes `create_user` succeeding after `create_pool(..., true)` alone and the tables absent after `create_pool(..., false)`.
