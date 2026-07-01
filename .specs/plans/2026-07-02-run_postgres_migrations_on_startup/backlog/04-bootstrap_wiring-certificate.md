# Done Certificate — Task 04: bootstrap wiring

**Task:** [04-bootstrap_wiring.md](04-bootstrap_wiring.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified

> This certificate is a verification protocol for Task 04. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 04) ≡ every obligation O1…O5 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** Both bootstrap Postgres call sites pass `pg_cfg.run_migrations.unwrap_or(true)` into `create_pool`, so a `postgres`-configured server migrates on startup across the user and session pools.
- **P2 — Obligations.** The task is done iff O1…O5 all hold. One Oi per definition-of-done item, in DoD order; O5 is the `Reviewable:` item.
- **P3 — Invariants.** Must not break the other repository arms (`dynamodb`, `sqlite`) in `build_user_repository`/`build_session_repository`, nor the `[repository.postgres]`-missing error path.

## Obligations

- **O1 — Both call sites pass `pg_cfg.run_migrations.unwrap_or(true)`; no placeholder literal remains.**
  - *Claim:* `build_user_repository` (`bootstrap.rs:185`) and `build_session_repository` (`bootstrap.rs:244`) both thread `pg_cfg.run_migrations.unwrap_or(true)` into `create_pool`.
  - *Evidence to collect:* read `crates/server/src/bootstrap.rs` around lines 185-189 and 244-248; confirm each `create_pool(...)` call's third argument is `pg_cfg.run_migrations.unwrap_or(true)` and that no hard-coded `true`/`false` from Task 02's placeholder remains at either site.
  - *Checks:* resolve `pg_cfg` to the `config.repository.postgres` binding in each arm (each arm resolves `[repository.postgres]` independently); confirm `unwrap_or(true)` — not `unwrap_or(false)` — encodes the "absent → migrate" default.
  - *Status:* ☐ unverified

- **O2 — Default migrates; `run_migrations = false` runs no DDL on either pool.**
  - *Claim:* with `run_migrations` absent/`true`, both pools migrate on startup; with `false`, neither runs DDL.
  - *Evidence to collect:* trace both arms: `run_migrations.unwrap_or(true)` yields `true` when the field is `None` or `Some(true)` and `false` when `Some(false)`, and that boolean reaches `create_pool`'s migration guard from Task 02.
  - *Status:* ☐ unverified

- **O3 — Coverage of the postgres bootstrap path.**
  - *Claim:* the `postgres` adapter path is exercised — a test asserts the repository is serviceable after startup, or the manual boot check is documented when `DATABASE_URL` is unset.
  - *Evidence to collect:* locate the bootstrap/integration test (or the extended Task 02 gated test); run it with `DATABASE_URL` set — expect the postgres path to build a serviceable repository; where no automated coverage exists, confirm the task/PR documents the manual boot check.
  - *Status:* ☐ unverified

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass, format and lint clean, ≥2 assertions per touched function, limits named.
  - *Evidence to collect:* from `.specs/development-guidelines.md` §Definition of done, run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, and `cargo nextest run --workspace` — expect all clean.
  - *Status:* ☐ unverified

- **O5 — Reviewable: postgres server serves after startup; `run_migrations = false` still connects (Reviewable).**
  - *Claim:* a reviewer boots the server with `repository.adapter = "postgres"` against a fresh DB and a repository-touching request succeeds (no 500); with `run_migrations = false` against the already-migrated DB, startup still connects.
  - *Evidence to collect:* start the server with a `postgres` repository config pointed at a scratch DB; issue a request that reads/writes the repository — observe no 500; set `run_migrations = false`, restart against the migrated DB — observe a clean startup.
  - *Status:* ☐ unverified

## Regression check

- `build_user_repository` / `build_session_repository` other arms: the `sqlite` and `dynamodb` arms are unchanged. Trace: an existing `sqlite` config still builds its repository via `sqlite::create_pool` unaffected : ☐ (PRESERVED / REGRESSION)
- The `[repository.postgres]`-missing `ConfigError` path (`bootstrap.rs:178-184`, `:237-243`) still returns its error when the section is absent : ☐ (PRESERVED / REGRESSION)

## Residue

- The second `create_pool` (session pool) re-running the DDL is a deliberate no-op given idempotent `IF NOT EXISTS` DDL; not a defect. Noted for the validator so a duplicate-migration observation is not mistaken for a bug.

## Conclusion

VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
