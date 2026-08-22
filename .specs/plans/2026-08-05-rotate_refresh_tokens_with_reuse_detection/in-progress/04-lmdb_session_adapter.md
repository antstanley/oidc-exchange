# Task 04 — LMDB session adapter

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted (done certificates are forbidden)

**Implements:** source spec §Session-only stores (LMDB), SR1–SR5, and LMDB reaper batching.
**Depends on:** 01 · domain_config_port_contract; 02 · shared_session_contract_harness
**Produces:** LMDB retired-token/family-index databases, one-write-transaction rotation, complete family revocation, bounded cleanup batching, and conformance coverage.
**Pointers:** `crates/adapters/src/lmdb/mod.rs`; LMDB tests.

## Steps

- [ ] Add `retired_tokens` and `family_index` named databases alongside sessions and user sessions.
- [ ] Resolve live/retired/superseded state and perform CAS read/delete/retirement/replacement/index updates in one heed write transaction.
- [ ] Implement family revocation across live and retired index entries; ensure revoke-all handles all families.
- [ ] Rework cleanup to delete sessions and retirement records in named `LMDB_CLEANUP_BATCH_SIZE` (256) write-transaction batches.
- [ ] Run the shared suite and add a high-occupancy cleanup regression that demonstrates batched deletion avoids a monolithic `MDB_MAP_FULL` path.

## Definition of done

- [ ] No externally visible rotation state is committed unless all three swap effects and indexes commit together.
- [ ] Family and user revocation remove every indexed live/retired record and return/report complete work.
- [ ] Expired retirement records are reaped; live entries survive cleanup; batch size is a named constant.
- [ ] Shared SR1–SR5 suite and LMDB-specific full-map/negative cleanup tests pass.
- [ ] Done certificates remain intentionally absent.
