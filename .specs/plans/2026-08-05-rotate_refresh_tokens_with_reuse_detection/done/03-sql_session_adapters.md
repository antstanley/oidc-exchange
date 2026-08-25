# Task 03 — PostgreSQL and SQLite session adapters

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted (done certificates are forbidden)

**Implements:** source spec §PostgreSQL / SQLite persistence and SR1–SR5.
**Depends on:** 01 · domain_config_port_contract; 02 · shared_session_contract_harness
**Produces:** SQL migration/read/write paths for live and retired generations, transactional CAS rotation, complete family revocation, and shared conformance invocation.
**Pointers:** `crates/adapters/src/{postgres,sqlite}/mod.rs`; their migrations/DDL; adapter tests.

## Steps

- [x] Extend sessions with nullable legacy-compatible `family_id`, generation, and rotated timestamp; create indexed `retired_refresh_tokens` with idempotent Postgres/SQLite migrations.
- [x] Implement classification, `BEGIN … COMMIT` delete-count CAS rotation, family deletion/count, and cleanup over both tables; preserve original expiry.
- [x] Specify and test first redemption of nullable legacy rows according to the resolution chosen under this plan’s recorded open question; do not silently synthesize an undocumented family.
- [x] Invoke the shared suite for both adapters and add migration/transaction rollback tests.

## Definition of done

- [x] Rotation is one transaction: zero affected live-session rows returns `false` and rolls back retirement/replacement writes.
- [x] `revoke_family` removes live and retained records and returns the exact removal count; cleanup count includes both tables.
- [x] Existing rows migrate without destructive backfill; successful reads/writes revalidate required stored state.
- [x] PostgreSQL and SQLite each run shared SR1–SR5 coverage plus local migration negative tests.
- [x] Done certificates remain intentionally absent.

## Completion notes

- Audited against the code on 2026-08-22 (wave B recovery): every criterion held as implemented; no gaps found. Both adapters share the same shapes, so notes cover them together (`postgres/mod.rs`, `sqlite/mod.rs`).
- Schema: `sessions` gains nullable `family_id`, `generation INTEGER NOT NULL DEFAULT 0`, `rotated_at` via the same idempotent step pattern as `users.version` (`ADD COLUMN IF NOT EXISTS` on Postgres; pragma-probed bare `ADD COLUMN` on SQLite, since it has no `IF NOT EXISTS` form). `retired_refresh_tokens` matches the source-spec DDL with the family and expires-at indexes; every statement is `IF [NOT] EXISTS` and safe to re-run.
- Rotation is one `BEGIN … COMMIT`: SELECT the live row inside the transaction (its existence plus the DELETE's affected-row count is the CAS), delete, conditionally insert the retirement record, insert the replacement plainly (a colliding hash fails loudly instead of clobbering), commit. Zero affected rows rolls everything back and returns `false`.
- Legacy-row resolution (the plan's recorded open question, chosen here and mirrored by tasks 04–06): a row whose `family_id` reads as the *empty string* sentinel on load is pre-rotation. Its classification stays storage-factual (`Live`, generation 0). Its first redemption swaps atomically but writes **no retirement record** — there is no prior generation to detect reuse against — and never synthesizes a family: the replacement carries whatever `fam_…` id the caller minted. Negative tests in both adapters prove the swap, the missing record, and the failed-CAS-nothing-written case.
- `revoke_family` deletes live rows and retirement records by `family_id` in one transaction and returns the summed affected count; `revoke_all_user_sessions` does the same by owner; `cleanup_expired_sessions` sweeps and counts both tables. The retirement deadline stamped per record is `min(retired_at + reuse_retention, family expires_at)` via `RetiredRefreshToken::retention_deadline`, so expiry is preserved from the family, never recomputed.
- Round-trip revalidation: session and record loaders parse every timestamp and status through typed conversions that surface corruption as `Error::StoreError`; `retirement_record` asserts the presented-hash and valid-family preconditions so a legacy row can never reach it.
- Coverage: each adapter runs `session_contract::assert_full_conformance` (tags `sqlite-session-conformance`, `postgres-session-conformance`) plus adapter-local negative tests — legacy-table migration upgrade + idempotence, legacy first-redemption swap and its failed CAS, mid-transaction replacement-collision rollback, two-table cleanup counting, per-user retired-record sweep scoping, expired-record-reads-Unknown-before-cleanup (SQLite) / isolated-schema variants of the same (Postgres, `#[ignore]`-gated behind `test_database_url` per its existing integration gating).
- Gates at completion: fmt clean, clippy `-D warnings` clean, nextest 432 passed / 43 skipped (Postgres and Dynamo suites inside the skip set, environment-gated). No done certificate exists.
