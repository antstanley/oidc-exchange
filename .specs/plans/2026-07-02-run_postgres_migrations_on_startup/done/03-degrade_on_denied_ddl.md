# Task 03 — degrade on denied DDL

**Plan:** [plan.md](../plan.md) · **Certificate:** [03-degrade_on_denied_ddl-certificate.md](03-degrade_on_denied_ddl-certificate.md)

**Implements:** [08-persistence.md](../../../service/specs/08-persistence.md) §PostgreSQL (`adapters/postgres`) — when the role lacks DDL rights and the migration is denied, the adapter logs a warning and proceeds if `users` and `sessions` already exist; startup fails only when they don't
**Depends on:** 02
**Produces:** on a migration denied with `SQLSTATE 42501` (insufficient_privilege), `create_pool` logs a structured warning and probes for the expected tables (`to_regclass('users')`, `to_regclass('sessions')`), returning the pool when both exist and failing with the original migration error when either is missing; non-permission migration errors still fail fast
**Pointers:** `crates/adapters/src/postgres/mod.rs` `create_pool` (the migration branch added in task 02); `sqlx::Error::as_database_error().code()` for the SQLSTATE; spec page `.specs/service/specs/08-persistence.md:34-38` (PostgreSQL section)

## Steps

- [x] Introduce a named constant for the Postgres insufficient-privilege SQLSTATE (e.g. `const INSUFFICIENT_PRIVILEGE_SQLSTATE: &str = "42501";`) in `crates/adapters/src/postgres/mod.rs`.
- [x] Wrap the `sqlx::raw_sql(MIGRATIONS)` execution from task 02: on error, classify via `err.as_database_error().and_then(|e| e.code())`. Only a code equal to the SQLSTATE constant enters the degrade branch; every other error is returned unchanged (fail fast).
- [x] In the degrade branch, log a structured `tracing::warn!` naming the denied-DDL condition, then probe `SELECT to_regclass('users'), to_regclass('sessions')`; return the pool when both regclasses are non-null, otherwise return the original migration error.
- [x] Add a unit test for the error-classification helper: a `42501` database error is routed to the degrade path, and a non-`42501` error is returned as fail-fast. (Factor the SQLSTATE check into a small pure helper so it is testable without a live restricted role; the table-probe path is exercised manually per the Reviewable line.)
- [x] Update `08-persistence.md` §PostgreSQL to document the degrade-with-warning behaviour on denied DDL (warn + proceed if the tables exist, fail only when they don't); bump the page's `**Date:**`.

## Definition of done

- [x] A migration failing with `SQLSTATE 42501` triggers the degrade branch (warn + `to_regclass` probe) while every non-`42501` failure is returned unchanged; the SQLSTATE literal is a named constant, not inline.
- [x] The degrade branch returns the pool when both `users` and `sessions` regclasses resolve and returns the original migration error when either is missing.
- [x] Negative-space test: the error classifier routes a non-`42501` error to fail-fast (and a `42501` error to the degrade path), proving the branch is genuinely conditional on the SQLSTATE.
- [x] `08-persistence.md` §PostgreSQL documents the degrade-with-warning path; the page `**Date:**` is bumped.
- [x] Meets the repo definition of done (tests, `cargo fmt`, `cargo clippy --workspace -- -D warnings`, ≥2 assertions per touched function, named-constant limits — see plan.md baseline).
- [x] Reviewable: a reviewer runs `create_pool(..., true)` under a role without DDL rights against a schema where `users`/`sessions` already exist and observes a logged warning and a successful startup, then drops one table and observes startup failing with the original error.
