# Done Certificate — Task 04: bootstrap wiring

**Task:** [04-bootstrap_wiring.md](04-bootstrap_wiring.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

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
  - *Status:* ☑ SATISFIED — the file has grown since authoring, so the call sites now sit at `bootstrap.rs:368-372` (`build_user_repository`) and `:428-432` (`build_session_repository`); both pass `pg_cfg.run_migrations.unwrap_or(true)` as `create_pool`'s third argument, the Task-02 placeholder `true` and its TODO comments are gone (per `jj diff`), each arm resolves its own `pg_cfg` from `config.repository.postgres.as_ref()`, and the fully-qualified `oidc_exchange_adapters::postgres::create_pool` resolves to `crates/adapters/src/postgres/mod.rs:147` (`url, max_connections, run_migrations: bool`) — not the sqlite `create_pool`, no shadowing.

- **O2 — Default migrates; `run_migrations = false` runs no DDL on either pool.**
  - *Claim:* with `run_migrations` absent/`true`, both pools migrate on startup; with `false`, neither runs DDL.
  - *Evidence to collect:* trace both arms: `run_migrations.unwrap_or(true)` yields `true` when the field is `None` or `Some(true)` and `false` when `Some(false)`, and that boolean reaches `create_pool`'s migration guard from Task 02.
  - *Status:* ☑ SATISFIED — trace: `unwrap_or(true)` → `true` for `None`/`Some(true)`, `false` for `Some(false)`; the boolean is `create_pool`'s `run_migrations` parameter, guarded at `crates/adapters/src/postgres/mod.rs:157` (`if run_migrations { sqlx::raw_sql(MIGRATIONS)… }`). Confirmed live against a scratch Postgres (docker `postgres:16-alpine`): field absent → fresh DB got `users`+`sessions` tables on boot; `run_migrations = false` against a second fresh DB → server started and `\dt` showed "Did not find any relations" (no DDL from either pool).

- **O3 — Coverage of the postgres bootstrap path.**
  - *Claim:* the `postgres` adapter path is exercised — a test asserts the repository is serviceable after startup, or the manual boot check is documented when `DATABASE_URL` is unset.
  - *Evidence to collect:* locate the bootstrap/integration test (or the extended Task 02 gated test); run it with `DATABASE_URL` set — expect the postgres path to build a serviceable repository; where no automated coverage exists, confirm the task/PR documents the manual boot check.
  - *Status:* ☑ SATISFIED — new `bootstrap::postgres_bootstrap_tests::postgres_bootstrap_migrates_both_pools_and_is_serviceable_on_startup` (crates/server/src/bootstrap.rs) builds both repositories through `AppConfig` with `run_migrations: None` and round-trips a user then a session; ran it with `DATABASE_URL` pointed at a fresh scratch DB → PASS (1 test run: 1 passed). Gated on `DATABASE_URL` with a clean skip, and the doc comment documents the manual boot check for when it is unset.

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass, format and lint clean, ≥2 assertions per touched function, limits named.
  - *Evidence to collect:* from `.specs/development-guidelines.md` §Definition of done, run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, and `cargo nextest run --workspace` — expect all clean.
  - *Status:* ☑ SATISFIED — `cargo fmt --check` exit 0; `cargo clippy --workspace -- -D warnings` clean; `cargo nextest run --workspace` → 284 passed, 0 failed (27 skipped). The new gated test carries 4 `assert_eq!`s plus expect-messages on every fallible step (≥2 assertions); no new magic numbers (the `unwrap_or(true)` default is the documented contract).

- **O5 — Reviewable: postgres server serves after startup; `run_migrations = false` still connects (Reviewable).**
  - *Claim:* a reviewer boots the server with `repository.adapter = "postgres"` against a fresh DB and a repository-touching request succeeds (no 500); with `run_migrations = false` against the already-migrated DB, startup still connects.
  - *Evidence to collect:* start the server with a `postgres` repository config pointed at a scratch DB; issue a request that reads/writes the repository — observe no 500; set `run_migrations = false`, restart against the migrated DB — observe a clean startup.
  - *Status:* ☑ SATISFIED — exercised live: booted `target/debug/oidc-exchange` (role `admin`, `repository.adapter = "postgres"`, internal API enabled) against a fresh scratch DB with `run_migrations` absent; startup logs show both pools migrating (second run all idempotent "already exists, skipping" notices — the Residue, not a bug); `POST /internal/users` returned **201** with the created user. Then set `run_migrations = false` in `[repository.postgres]` and restarted against that migrated DB: clean startup with zero migration notices, and `GET /internal/users` returned **200**.

## Regression check

- `build_user_repository` / `build_session_repository` other arms: the `sqlite` and `dynamodb` arms are unchanged. Trace: an existing `sqlite` config still builds its repository via `sqlite::create_pool` unaffected : ☑ PRESERVED — the diff touches only the two `postgres` arms (placeholder `true` → config read) plus the new test module; `sqlite` arms at `bootstrap.rs:378-392`/`:438-450` and `dynamodb` arms are byte-identical, and the full workspace suite (which drives sqlite-backed bootstrap paths) is green.
- The `[repository.postgres]`-missing `ConfigError` path (`bootstrap.rs:178-184`, `:237-243`) still returns its error when the section is absent : ☑ PRESERVED — now at `bootstrap.rs:361-367`/`:421-427`, untouched by the diff; each arm still `ok_or_else`-returns its `ConfigError` before any pool is built.

## Residue

- The second `create_pool` (session pool) re-running the DDL is a deliberate no-op given idempotent `IF NOT EXISTS` DDL; not a defect. Noted for the validator so a duplicate-migration observation is not mistaken for a bug.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O5 all SATISFIED with direct evidence — both call sites read `pg_cfg.run_migrations.unwrap_or(true)` (no placeholder literal remains), the gated bootstrap test passed against a fresh scratch Postgres, fmt/clippy/nextest are clean (284 passed), and the Reviewable boot check was exercised live (fresh-DB boot → 201 on a repository write; `run_migrations = false` → clean startup, no DDL, 200) — both regression traces PRESERVED. Note: the certificate's authored line pointers (185-189/244-248) predate later merges; the arms now sit at 368-372/428-432, same code, no drift in substance.
