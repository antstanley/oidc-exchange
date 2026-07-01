# Done Certificate — Task 02: migrate on startup

**Task:** [02-migrate_on_startup.md](02-migrate_on_startup.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified

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
  - *Status:* ☐ unverified

- **O2 — Gated integration test proves fresh-DB service and the false-path emptiness.**
  - *Claim:* with `DATABASE_URL` set, a test shows `create_user` succeeds after `create_pool(url, n, true)` alone, and `create_pool(url, n, false)` leaves a fresh schema with no tables; with `DATABASE_URL` unset the test skips.
  - *Evidence to collect:* locate the gated Postgres test; run `cargo nextest run --workspace` with `DATABASE_URL` set to a scratch Postgres — expect the test to PASS; run again with `DATABASE_URL` unset — expect the test to be skipped, not failed.
  - *Checks:* confirm the skip is a genuine early-return/skip on the missing env var, not a silently-passing empty test.
  - *Status:* ☐ unverified

- **O3 — Negative-space: `run_migrations = false` leaves no tables.**
  - *Claim:* after `create_pool(url, n, false)` on a fresh schema, `to_regclass('users')` and `to_regclass('sessions')` are null.
  - *Evidence to collect:* in the gated test, confirm the false-path assertion probes for the tables' absence (e.g. `to_regclass` is null or a `SELECT` against `users` errors with undefined_table); run it against a fresh schema with `DATABASE_URL` set — expect PASS.
  - *Status:* ☐ unverified

- **O4 — 08-persistence.md documents migrate-on-startup and connect-only.**
  - *Claim:* `08-persistence.md` §PostgreSQL states `create_pool` runs the idempotent migrations before returning unless `run_migrations = false`, in which case it only connects; the page `**Date:**` is bumped.
  - *Evidence to collect:* read `.specs/service/specs/08-persistence.md` §PostgreSQL (around lines 34-38); confirm the migrate-on-startup and `run_migrations = false` prose is present and the header `**Date:**` differs from `2026-06-24`.
  - *Status:* ☐ unverified

- **O5 — Meets the repo definition of done.**
  - *Claim:* tests pass, format and lint clean, ≥2 assertions per touched function, limits named.
  - *Evidence to collect:* from `.specs/development-guidelines.md` §Definition of done, run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, and `cargo nextest run --workspace` — expect all clean; confirm the new/touched test functions carry ≥2 meaningful assertions.
  - *Status:* ☐ unverified

- **O6 — Reviewable: gated test shows create_user after migrate and no tables after connect-only (Reviewable).**
  - *Claim:* a reviewer runs the gated test against a scratch Postgres and observes `create_user` succeeding after `create_pool(..., true)` alone and the tables absent after `create_pool(..., false)`.
  - *Evidence to collect:* `DATABASE_URL=<scratch> cargo nextest run -p oidc-exchange-adapters <test name>`; observe the create-user-after-migrate assertion and the no-tables-after-false assertion both passing.
  - *Status:* ☐ unverified

## Regression check

- `crates/server/src/bootstrap.rs:185` and `:244` call `create_pool`. Trace: after the signature gains `run_migrations: bool`, both call sites compile (task 02 passes a placeholder `true`) and the server still builds : ☐ (PRESERVED / REGRESSION)
- The SQLite `create_pool` (`crates/adapters/src/sqlite/mod.rs:54-79`) is untouched and its existing tests still pass : ☐ (PRESERVED / REGRESSION)

## Residue

- The denied-DDL degrade branch (`SQLSTATE 42501`) is Task 03, not an obligation here — task 02's migration failure path returns the raw `sqlx::Error`.
- The placeholder `true` passed at the bootstrap call sites is replaced by the config-driven value in Task 04.

## Conclusion

VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
