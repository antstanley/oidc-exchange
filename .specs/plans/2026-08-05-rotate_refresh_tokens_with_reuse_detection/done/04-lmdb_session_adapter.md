# Task 04 — LMDB session adapter

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted (done certificates are forbidden)

**Implements:** source spec §Session-only stores (LMDB), SR1–SR5, and LMDB reaper batching.
**Depends on:** 01 · domain_config_port_contract; 02 · shared_session_contract_harness
**Produces:** LMDB retired-token/family-index databases, one-write-transaction rotation, complete family revocation, bounded cleanup batching, and conformance coverage.
**Pointers:** `crates/adapters/src/lmdb/mod.rs`; LMDB tests.

## Steps

- [x] Add `retired_tokens` and `family_index` named databases alongside sessions and user sessions.
- [x] Resolve live/retired/superseded state and perform CAS read/delete/retirement/replacement/index updates in one heed write transaction.
- [x] Implement family revocation across live and retired index entries; ensure revoke-all handles all families.
- [x] Rework cleanup to delete sessions and retirement records in named `LMDB_CLEANUP_BATCH_SIZE` (256) write-transaction batches.
- [x] Run the shared suite and add a high-occupancy cleanup regression that demonstrates batched deletion avoids a monolithic `MDB_MAP_FULL` path.

## Definition of done

- [x] No externally visible rotation state is committed unless all three swap effects and indexes commit together.
- [x] Family and user revocation remove every indexed live/retired record and return/report complete work.
- [x] Expired retirement records are reaped; live entries survive cleanup; batch size is a named constant.
- [x] Shared SR1–SR5 suite and LMDB-specific full-map/negative cleanup tests pass.
- [x] Done certificates remain intentionally absent.

## Completion notes

- Audited against the code on 2026-08-22 (wave B recovery): every criterion held as implemented; no gaps found (`crates/adapters/src/lmdb/mod.rs`).
- Four named databases opened in one bootstrap write transaction: `sessions` (hash → session JSON), `user_sessions` (`{user_id}:{hash}` → hash), `retired_tokens` (hash → record JSON), `family_index` (`{family_id}\0{hash}` → `live`|`retired`, with kind constants).
- Rotation runs entirely inside one heed write transaction on `spawn_blocking`: CAS is the live row's presence read inside that transaction; the swap deletes the live entry plus both index filings, writes the retirement record + `family_index` retired filing, installs the replacement + its two index filings, and commits — or drops the transaction having written nothing. Replacement-hash collisions with an existing live row or retired record are asserted as caller bugs rather than silently overwritten. Legacy rows (empty-family sentinel) swap without a retirement record, exactly as task 03 resolved.
- Classification reads live then retained state inside read transactions, evaluating the successor pointer for `Superseded` vs `Retired`; expired records answer `Unknown`.
- `revoke_family` walks the family_index range in one write transaction removing both kinds of entries and returns the exact count; `revoke_all_user_sessions` walks the user's `user_sessions` prefix for live generations and scans `retired_tokens` by owner for retained records, deleting everything in one committed transaction. Both assert index/session agreement while iterating (a dangling index entry is corruption, not silence).
- Cleanup collects expired sessions and records, then deletes in committed batches of `LMDB_CLEANUP_BATCH_SIZE = 256`; a batch that hits `MDB_MAP_FULL` halves its width up to `CLEANUP_MAX_BATCH_HALVINGS = 8` (down to single-delete transactions) instead of failing the sweep. The count covers both tables; live sessions are never candidates.
- The high-occupancy regression fills a 1 MB map past 90% occupancy with more than one batch of expired entries and proves the batched sweep completes, reports every removal, spans multiple committed batches, and leaves the map writable (a fresh session stores and classifies afterwards). A deliberately-monolithic control is documented as intentionally absent: within-transaction page reuse makes "monolithic fails" only true at occupancy where no transaction of any width can run, so the test pins the property production relies on rather than an unassertable wedge.
- Coverage: `session_contract::assert_full_conformance(&store, "lmdb-session-conformance")` plus adapter-local negative tests (legacy first-redemption swap and failed CAS, two-table cleanup counting, live-survives-cleanup, per-user retired sweep scoping, near-capacity batching). All normal-tier tests run on every build.
- Gates at completion: fmt clean, clippy `-D warnings` clean, nextest 432 passed / 43 skipped. No done certificate exists.
