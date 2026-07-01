# Change: Run Postgres migrations on startup

**Status:** Proposed · **Date:** 2026-07-01 · **Owner:** Ant Stanley · **Target:** crates/adapters, crates/server

Execute the Postgres adapter's `MIGRATIONS` DDL inside `create_pool`, matching the SQLite
adapter, so a fresh Postgres deployment has its `users` and `sessions` tables created on
startup instead of 500ing on every request.

---

## Motivation

`crates/adapters/src/postgres/mod.rs` defines `pub const MIGRATIONS` that is never executed
anywhere in the repository; `create_pool` only builds the connection pool. The SQLite adapter
runs its migrations inside `create_pool`, and the deployment guide
(`docs/deployment/linux-postgres.md`, line 57) promises "oidc-exchange runs its own migrations
on startup". A fresh Postgres deployment following that guide starts a server with no tables —
every request that touches the repository fails.

The DDL is already idempotent (`CREATE TABLE IF NOT EXISTS`, `CREATE INDEX IF NOT EXISTS`
throughout), so running it on every startup is safe, including when the bootstrap builds two
pools (user and session repositories both configured as `postgres`).

---

## Affected spec pages

| Canonical page                                                                 | Nature of change                                                                  |
| ------------------------------------------------------------------------------ | --------------------------------------------------------------------------------- |
| [`.specs/service/specs/08-persistence.md`](../service/specs/08-persistence.md) | PostgreSQL section: `create_pool` runs the idempotent migrations before returning; degrade-with-warning on denied DDL |
| [`.specs/service/specs/06-configuration.md`](../service/specs/06-configuration.md) | `[repository]` section: add `run_migrations?` to the `[repository.postgres]` keys |

---

## Proposed changes

### `.specs/service/specs/08-persistence.md` → PostgreSQL (`adapters/postgres`) (Modify)

> Two tables via `sqlx`: `users` and `sessions`. `metadata` and `claims` are `JSONB`; indexes
> cover `(external_id, provider)` and `sessions.user_id`. `create_pool` builds the connection
> pool and, unless `[repository.postgres] run_migrations = false`, executes the adapter's
> idempotent migrations (`CREATE TABLE IF NOT EXISTS …`) before returning — like SQLite, a
> fresh database is ready to serve after startup with no external migration step. When the
> role lacks DDL rights and the migration is denied, the adapter logs a warning and proceeds
> if the `users` and `sessions` tables already exist; startup fails only when they don't.
> With `run_migrations = false`, `create_pool` only connects. Implements both repository
> traits.

### `.specs/service/specs/06-configuration.md` → `[repository]` (users + sessions) (Modify)

> `adapter` (`dynamodb` | `postgres` | `sqlite`), with one of
> `[repository.dynamodb] { table_name, region? }`,
> `[repository.postgres] { url, max_connections?, run_migrations? }`,
> `[repository.sqlite] { path }`. `run_migrations` defaults to `true`; set it to `false` for
> locked-down databases where the app role has no DDL rights and migrations are applied
> out-of-band.

---

## Type changes

`PostgresConfig` (`crates/core/src/config.rs:149-152`) gains `run_migrations: Option<bool>`
(absent → `true`). Config types are not in the canonical-types schema sidecar, so no schema
change.

---

## Implementation notes

1. `crates/adapters/src/postgres/mod.rs:60-68` — after `connect`, when `run_migrations` is
   true, execute `MIGRATIONS` (defined at `postgres/mod.rs:14-42`). The constant is a
   multi-statement block; prepared statements cannot run multiple statements on Postgres, so
   use sqlx's raw execution path (`sqlx::raw_sql(MIGRATIONS).execute(&pool)`, simple-query
   protocol, sqlx 0.8).
2. Degrade-with-warning path: if the migration fails with a Postgres permission error
   (SQLSTATE `42501` insufficient_privilege on the database error), log a warning and probe
   for the expected tables (e.g. `SELECT to_regclass('users'), to_regclass('sessions')`);
   proceed when both exist, otherwise fail startup with the original error. Non-permission
   errors still fail fast.
3. Mirror the SQLite shape at `crates/adapters/src/sqlite/mod.rs:55-79` (migrations inside
   `create_pool`); `create_pool` gains a `run_migrations: bool` parameter and keeps returning
   `Result<PgPool, sqlx::Error>`.
4. `crates/core/src/config.rs:149-152` — add `run_migrations: Option<bool>` to
   `PostgresConfig` (absent → `true`).
5. Bootstrap: both call sites (`crates/server/src/bootstrap.rs:185` and `:244`) pass
   `pg_cfg.run_migrations.unwrap_or(true)` through `create_pool`, so each pool migrates;
   idempotent DDL makes the second run a no-op.
6. Extend the Postgres integration tests (or add one gated on a `DATABASE_URL`/testcontainers
   environment) asserting a fresh database serves `create_user` after `create_pool` alone,
   and that `run_migrations = false` leaves a fresh database without tables.

---

## Merge plan

1. Apply the `Proposed changes` blocks to
   [08-persistence.md](../service/specs/08-persistence.md) and
   [06-configuration.md](../service/specs/06-configuration.md); bump each page's `**Date:**`.
2. No schema change.
3. Flip `**Status:**` to `Merged`, stamp `**Merged:**`, move to `.specs/changes/merged/`.
4. Update `.specs/README.md`.

---

## Assumptions and open questions

### Assumptions

- The configured Postgres role has DDL rights on its schema for the documented path (the
  deployment guide creates the database with `OWNER oidc_exchange`); locked-down deployments
  either set `run_migrations = false` or rely on the degrade-with-warning path.
- Schema evolution remains additive `IF NOT EXISTS` DDL for now; versioned migrations
  (`sqlx::migrate!`) are not introduced by this change.

### Decisions

- _Migrate inside `create_pool`, not in bootstrap._ **Matches SQLite and keeps the adapter
  self-contained** — any embedder that builds a pool gets a working schema.
- _Config flag for locked-down databases._ **`[repository.postgres] run_migrations` (default
  `true`); when `false`, `create_pool` only connects.** Deployments that apply DDL out-of-band
  can run the app under a role with no DDL rights.
- _Degrade with a warning on denied DDL._ **A permission-denied migration logs a warning and
  startup proceeds if the expected tables exist, failing only when they don't.** A
  pre-provisioned schema under a restricted role keeps working even without the flag set.

### Open questions

- (None at this stage.)
