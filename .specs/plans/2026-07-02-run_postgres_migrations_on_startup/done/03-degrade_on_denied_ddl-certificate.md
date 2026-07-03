# Done Certificate — Task 03: degrade on denied DDL

**Task:** [03-degrade_on_denied_ddl.md](03-degrade_on_denied_ddl.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

> This certificate is a verification protocol for Task 03. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 03) ≡ every obligation O1…O6 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** On a migration denied with `SQLSTATE 42501`, `create_pool` logs a warning and probes for the expected tables, returning the pool when both exist and failing with the original error otherwise; non-permission errors still fail fast.
- **P2 — Obligations.** The task is done iff O1…O6 all hold. One Oi per definition-of-done item, in DoD order; O6 is the `Reviewable:` item.
- **P3 — Invariants.** Must not break the happy-path migration from Task 02 (a successful `raw_sql(MIGRATIONS)` still returns the migrated pool) nor the `run_migrations = false` connect-only path.

## Obligations

- **O1 — `42501` triggers the degrade branch; other errors fail fast; SQLSTATE is a named constant.**
  - *Claim:* the migration error is classified via `as_database_error().and_then(|e| e.code())`; only a code equal to the named SQLSTATE constant enters the degrade branch, all others are returned unchanged.
  - *Evidence to collect:* read `crates/adapters/src/postgres/mod.rs` `create_pool`; confirm a named constant (e.g. `INSUFFICIENT_PRIVILEGE_SQLSTATE = "42501"`) exists and the branch compares the DB error code to it; confirm the non-matching arm returns the original `sqlx::Error`.
  - *Checks:* resolve `as_database_error()`/`.code()` to the `sqlx::Error` / `DatabaseError` API (sqlx 0.8), not a local shadow; confirm `"42501"` appears only in the constant definition, not inline at the comparison.
  - *Status:* ☑ SATISFIED — `crates/adapters/src/postgres/mod.rs:122` defines `const INSUFFICIENT_PRIVILEGE_SQLSTATE: &str = "42501"`; `create_pool` (mod.rs:158–162) classifies via `err.as_database_error().and_then(|db_err| db_err.code())` and routes through the pure helper `is_insufficient_privilege_code` (mod.rs:129–131), returning the original `sqlx::Error` unchanged (`return Err(err)`, mod.rs:161) for any non-matching code. Resolution: `as_database_error()`/`.code()` resolve to sqlx 0.8's `sqlx::Error`/`DatabaseError` methods (no local shadow; same API already used by `is_unique_violation`, mod.rs:112–116). `grep -n '42501'` shows the literal only at the constant definition (line 122); the other hits are doc comments and the test name, none inline at a comparison.

- **O2 — Degrade branch proceeds when both tables exist, fails with the original error otherwise.**
  - *Claim:* on `42501`, `create_pool` probes `to_regclass('users')`/`to_regclass('sessions')`; returns the pool when both are non-null, returns the original migration error when either is null.
  - *Evidence to collect:* read the degrade branch; confirm it runs the `to_regclass` probe and that the "either missing" path returns the *original* migration error (captured before the probe), not the probe's own error.
  - *Checks:* confirm the probe error itself (if the probe query fails) does not mask the original denied-DDL error in the failure path.
  - *Status:* ☑ SATISFIED — the degrade branch (mod.rs:170–193) runs `SELECT to_regclass('users')::text …, to_regclass('sessions')::text …`, requires both non-null (`users_reg.is_some() && sessions_reg.is_some()`), and on `!tables_exist` returns `Err(err)` — the original migration error captured by the `if let Err(err)` binding at mod.rs:158, not a probe error. Mask check: the probe result is folded via `.ok() … .unwrap_or(false)`, so a failed or inconclusive probe yields `tables_exist = false` and the original error is returned (comment at mod.rs:185–187 states this intent). Confirmed live in the O6 trace: with `sessions` dropped, startup failed with the original 42501 migration error, not a probe error.

- **O3 — Negative-space test: classifier routes non-`42501` to fail-fast and `42501` to degrade.**
  - *Claim:* the SQLSTATE-classification helper returns fail-fast for a non-`42501` code and degrade for `42501`.
  - *Evidence to collect:* run the classifier unit test — expect PASS on both the `42501` (degrade) and non-`42501` (fail-fast) cases; confirm the helper is pure (takes a code/error, returns the routing decision) so it runs without a live restricted role.
  - *Status:* ☑ SATISFIED — `cargo nextest run -p oidc-exchange-adapters is_insufficient_privilege` → PASS: `postgres::tests::is_insufficient_privilege_code_routes_only_42501_to_degrade` (mod.rs:1158–1172) asserts `Some("42501")` → true (degrade), `Some("42P01")` → false (fail-fast), and `None` → false (fail-fast) — 3 assertions. The helper is pure (`Option<&str>` in, `bool` out, no `sqlx::Error` dependency), so it runs without a live restricted role.

- **O4 — 08-persistence.md documents the degrade-with-warning path.**
  - *Claim:* `08-persistence.md` §PostgreSQL states that a denied migration logs a warning and proceeds if the tables exist, failing only when they don't; the page `**Date:**` is bumped.
  - *Evidence to collect:* read `.specs/service/specs/08-persistence.md` §PostgreSQL; confirm the degrade-with-warning prose is present and the header `**Date:**` reflects this change.
  - *Status:* ☑ SATISFIED — `.specs/service/specs/08-persistence.md:94–99` (§PostgreSQL) now documents the SQLSTATE `42501` degrade path: structured warning, `to_regclass('users')`/`to_regclass('sessions')` probe, pool returned when both exist, original migration error when either is missing, every other failure still fail-fast. The header `**Date:** 2026-07-02` already reads this change's date (Task 02 bumped it the same day), so it correctly reflects the change; no further bump was possible or needed.

- **O5 — Meets the repo definition of done.**
  - *Claim:* tests pass, format and lint clean, ≥2 assertions per touched function, the SQLSTATE limit named.
  - *Evidence to collect:* from `.specs/development-guidelines.md` §Definition of done, run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, and `cargo nextest run --workspace` — expect all clean; confirm the classifier test carries ≥2 meaningful assertions.
  - *Status:* ☑ SATISFIED — `cargo fmt --check` clean (no output); `cargo clippy --workspace -- -D warnings` finished clean; `cargo nextest run --workspace` → 283 passed, 0 failed (27 skipped = gated live-service tests). The classifier test carries 3 meaningful assertions with messages; the new bound (`42501`) is the named constant `INSUFFICIENT_PRIVILEGE_SQLSTATE`.

- **O6 — Reviewable: warn-and-proceed under a restricted role with pre-existing tables; fail when a table is dropped (Reviewable).**
  - *Claim:* under a role without DDL rights against a schema where `users`/`sessions` already exist, `create_pool(..., true)` logs a warning and starts up; dropping one table makes startup fail with the original error.
  - *Evidence to collect:* run `create_pool(..., true)` against a scratch Postgres where the tables were pre-created, under a role lacking DDL rights — observe the `tracing::warn!` line and a returned pool; drop `sessions`, rerun — observe startup failing with the original migration error.
  - *Status:* ☑ SATISFIED — exercised live against a scratch `postgres:16` Docker container: superuser pre-ran `MIGRATIONS` in a fresh `appdb`, then a `restricted` LOGIN role (USAGE + DML grants, no DDL/ownership; `ALTER TABLE users …` as that role confirmed to raise `ERROR: 42501: must be owner of table users`). A harness binary calling `create_pool(url, 1, true)` as `restricted` printed `WARN … postgres migration denied (insufficient privilege); probing for pre-existing users/sessions tables sqlstate="42501"`, then `WARN … proceeding despite denied migration DDL: users/sessions tables already exist`, and returned `Ok(pool)`. After `DROP TABLE sessions` (superuser), the rerun logged the denied-DDL warning and failed with the original migration error (`error returned from database: permission denied for schema public`, SQLSTATE 42501) — not a probe error.

## Regression check

- `create_pool` happy path from Task 02: a successful migration (role has DDL rights) still returns the migrated pool without hitting the degrade branch. Trace the gated integration test from Task 02 — expect it still PASSES : ☑ PRESERVED — with `POSTGRES_TEST_URL` pointed at the scratch Postgres, `postgres::tests::create_pool_migrates_on_startup_and_run_migrations_false_stays_bare` PASSED (covering both the migrate-on-startup happy path and the `run_migrations = false` connect-only path per P3), as did all 12 postgres adapter tests under `--run-ignored=all` (12 passed, 0 failed).

## Residue

- The `to_regclass` probe path is exercised manually (Reviewable O6) rather than in an automated test, because it requires a role without DDL rights; noted as a follow-up if hermetic CI coverage of the restricted-role path is wanted.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O6 all SATISFIED with collected evidence — code inspection (named constant, pure classifier, original-error-preserving degrade branch), the passing classifier unit test, clean fmt/clippy/nextest (283 passed), the bumped spec page, and a live restricted-role exercise of the Reviewable scenario (warn-and-proceed with pre-existing tables, original 42501 error after dropping `sessions`) — and the Task 02 happy-path/connect-only regression surface is PRESERVED.
