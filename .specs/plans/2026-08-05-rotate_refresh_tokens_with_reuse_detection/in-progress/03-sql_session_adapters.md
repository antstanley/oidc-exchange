# Task 03 — PostgreSQL and SQLite session adapters

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted (done certificates are forbidden)

**Implements:** source spec §PostgreSQL / SQLite persistence and SR1–SR5.
**Depends on:** 01 · domain_config_port_contract; 02 · shared_session_contract_harness
**Produces:** SQL migration/read/write paths for live and retired generations, transactional CAS rotation, complete family revocation, and shared conformance invocation.
**Pointers:** `crates/adapters/src/{postgres,sqlite}/mod.rs`; their migrations/DDL; adapter tests.

## Steps

- [ ] Extend sessions with nullable legacy-compatible `family_id`, generation, and rotated timestamp; create indexed `retired_refresh_tokens` with idempotent Postgres/SQLite migrations.
- [ ] Implement classification, `BEGIN … COMMIT` delete-count CAS rotation, family deletion/count, and cleanup over both tables; preserve original expiry.
- [ ] Specify and test first redemption of nullable legacy rows according to the resolution chosen under this plan’s recorded open question; do not silently synthesize an undocumented family.
- [ ] Invoke the shared suite for both adapters and add migration/transaction rollback tests.

## Definition of done

- [ ] Rotation is one transaction: zero affected live-session rows returns `false` and rolls back retirement/replacement writes.
- [ ] `revoke_family` removes live and retained records and returns the exact removal count; cleanup count includes both tables.
- [ ] Existing rows migrate without destructive backfill; successful reads/writes revalidate required stored state.
- [ ] PostgreSQL and SQLite each run shared SR1–SR5 coverage plus local migration negative tests.
- [ ] Done certificates remain intentionally absent.
