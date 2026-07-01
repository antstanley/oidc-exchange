# Done Certificate — Task 03: degrade on denied DDL

**Task:** [03-degrade_on_denied_ddl.md](03-degrade_on_denied_ddl.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified

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
  - *Status:* ☐ unverified

- **O2 — Degrade branch proceeds when both tables exist, fails with the original error otherwise.**
  - *Claim:* on `42501`, `create_pool` probes `to_regclass('users')`/`to_regclass('sessions')`; returns the pool when both are non-null, returns the original migration error when either is null.
  - *Evidence to collect:* read the degrade branch; confirm it runs the `to_regclass` probe and that the "either missing" path returns the *original* migration error (captured before the probe), not the probe's own error.
  - *Checks:* confirm the probe error itself (if the probe query fails) does not mask the original denied-DDL error in the failure path.
  - *Status:* ☐ unverified

- **O3 — Negative-space test: classifier routes non-`42501` to fail-fast and `42501` to degrade.**
  - *Claim:* the SQLSTATE-classification helper returns fail-fast for a non-`42501` code and degrade for `42501`.
  - *Evidence to collect:* run the classifier unit test — expect PASS on both the `42501` (degrade) and non-`42501` (fail-fast) cases; confirm the helper is pure (takes a code/error, returns the routing decision) so it runs without a live restricted role.
  - *Status:* ☐ unverified

- **O4 — 08-persistence.md documents the degrade-with-warning path.**
  - *Claim:* `08-persistence.md` §PostgreSQL states that a denied migration logs a warning and proceeds if the tables exist, failing only when they don't; the page `**Date:**` is bumped.
  - *Evidence to collect:* read `.specs/service/specs/08-persistence.md` §PostgreSQL; confirm the degrade-with-warning prose is present and the header `**Date:**` reflects this change.
  - *Status:* ☐ unverified

- **O5 — Meets the repo definition of done.**
  - *Claim:* tests pass, format and lint clean, ≥2 assertions per touched function, the SQLSTATE limit named.
  - *Evidence to collect:* from `.specs/development-guidelines.md` §Definition of done, run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, and `cargo nextest run --workspace` — expect all clean; confirm the classifier test carries ≥2 meaningful assertions.
  - *Status:* ☐ unverified

- **O6 — Reviewable: warn-and-proceed under a restricted role with pre-existing tables; fail when a table is dropped (Reviewable).**
  - *Claim:* under a role without DDL rights against a schema where `users`/`sessions` already exist, `create_pool(..., true)` logs a warning and starts up; dropping one table makes startup fail with the original error.
  - *Evidence to collect:* run `create_pool(..., true)` against a scratch Postgres where the tables were pre-created, under a role lacking DDL rights — observe the `tracing::warn!` line and a returned pool; drop `sessions`, rerun — observe startup failing with the original migration error.
  - *Status:* ☐ unverified

## Regression check

- `create_pool` happy path from Task 02: a successful migration (role has DDL rights) still returns the migrated pool without hitting the degrade branch. Trace the gated integration test from Task 02 — expect it still PASSES : ☐ (PRESERVED / REGRESSION)

## Residue

- The `to_regclass` probe path is exercised manually (Reviewable O6) rather than in an automated test, because it requires a role without DDL rights; noted as a follow-up if hermetic CI coverage of the restricted-role path is wanted.

## Conclusion

VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
