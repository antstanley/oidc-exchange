# Task 04 — bootstrap wiring

**Plan:** [plan.md](../plan.md) · **Certificate:** [04-bootstrap_wiring-certificate.md](04-bootstrap_wiring-certificate.md)

**Implements:** [08-persistence.md](../../../service/specs/08-persistence.md) §PostgreSQL (`adapters/postgres`) — a fresh Postgres deployment is ready to serve after startup with no external migration step, across both the user and session pools
**Depends on:** 01, 02
**Produces:** both bootstrap call sites pass `pg_cfg.run_migrations.unwrap_or(true)` into `create_pool`, so a `postgres`-configured server migrates on startup — the user pool and (when sessions also use Postgres) the session pool each run the idempotent DDL, the second being a no-op
**Pointers:** `crates/server/src/bootstrap.rs:185-189` (`build_user_repository` Postgres arm) and `:244-248` (`build_session_repository` Postgres arm); `PostgresConfig.run_migrations` from task 01; `create_pool` signature from task 02

## Steps

- [x] In `build_user_repository`'s `postgres` arm (`bootstrap.rs:185`), replace the placeholder `run_migrations` argument with `pg_cfg.run_migrations.unwrap_or(true)`.
- [x] In `build_session_repository`'s `postgres` arm (`bootstrap.rs:244`), pass `pg_cfg.run_migrations.unwrap_or(true)` the same way, so the session pool also migrates (idempotent DDL makes the second run a no-op).
- [x] Confirm no placeholder `true`/`false` argument from task 02 remains at either call site — both read the config field.
- [x] Verify the `unwrap_or(true)` default matches the documented "absent → migrate" contract from task 01 and the SQLite parity in the change spec.

## Definition of done

- [x] Both Postgres call sites in `bootstrap.rs` pass `pg_cfg.run_migrations.unwrap_or(true)` into `create_pool`; no hard-coded `run_migrations` literal remains at either site.
- [x] With `run_migrations` absent (or `true`) in `[repository.postgres]`, a `postgres`-configured server migrates on startup; with `run_migrations = false`, neither pool runs DDL.
- [x] Coverage: a bootstrap/integration test (or the gated Postgres test extended from task 02) exercises the `postgres` adapter path and asserts the repository is serviceable after startup, or documents the manual boot check when `DATABASE_URL` is unset.
- [x] Meets the repo definition of done (tests, `cargo fmt`, `cargo clippy --workspace -- -D warnings`, ≥2 assertions per touched function, named-constant limits — see plan.md baseline).
- [x] Reviewable: a reviewer starts the server with `repository.adapter = "postgres"` against a fresh database and observes a request that touches the repository succeeding (no 500), then sets `run_migrations = false` against that same migrated database and observes startup still connecting.
