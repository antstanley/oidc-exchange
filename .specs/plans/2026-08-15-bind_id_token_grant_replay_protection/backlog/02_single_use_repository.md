# Task 02 — Single-use repository

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted at user request

**Implements:** [`.specs/changes/2026-08-05-bind_id_token_grant_replay_protection.md` §Proposed changes → `02-ports-and-adapters.md` and `08-persistence.md`](../../../changes/2026-08-05-bind_id_token_grant_replay_protection.md), [§Type changes](../../../changes/2026-08-05-bind_id_token_grant_replay_protection.md), [§Implementation notes 2](../../../changes/2026-08-05-bind_id_token_grant_replay_protection.md); [01-domain-model.md §ID scheme and §Required query patterns](../../../service/specs/01-domain-model.md), [02-ports-and-adapters.md §SessionRepository](../../../service/specs/02-ports-and-adapters.md), [08-persistence.md §DynamoDB, §PostgreSQL, §SQLite, and §Session-only stores](../../../service/specs/08-persistence.md)
**Depends on:** —
**Produces:** Every session repository atomically claims or consumes a digest-keyed, expiry-aware single-use record, including the in-memory test implementation.
**Pointers:** `crates/core/src/ports/repository.rs:24-34`, `crates/adapters/src/{dynamo,postgres,sqlite,lmdb,valkey}/`, `crates/test-utils/src/lib.rs:21-49`, `schemas/datamodel.schema.json`, `crates/adapters/src/lmdb/mod.rs:29`

## Steps

- [ ] Add `put_single_use(key, expires_at)` and `take_single_use(key)` to `SessionRepository`, documenting atomic insert-if-absent/remove-and-report behavior and expired-is-absent semantics; expand cleanup semantics to include these records.
- [ ] Add the logical `SingleUseRecord` representation and storage schema updates, keeping only namespaced digests and expiry in persistent state.
- [ ] Implement adapter-native atomic semantics for DynamoDB, Postgres, SQLite, Valkey, and LMDB, including SQL DDL/index migration, Dynamo/Valkey native expiry, and one additional LMDB named database with the appropriately raised database limit.
- [ ] Extend cleanup to reclaim expired single-use records where native expiry does not, while making both claim operations correct without a reaper.
- [ ] Extend `MockRepository` with shared atomic single-use state and write cross-adapter conformance tests covering first claim, duplicate claim, consume, expired-is-absent, cleanup, and concurrent contention.

## Definition of done

- [ ] `put_single_use` succeeds exactly once for one live key and treats an expired record as absent; `take_single_use` consumes a live record exactly once and never accepts an expired record.
- [ ] Two concurrent claims of one key produce exactly one success for every adapter, and adapter failures remain typed `StoreError` values rather than silent fallback.
- [ ] SQL/LMDB schema changes are idempotent and cleanup counts both sessions and single-use records without changing live-session behavior.
- [ ] New persistent fields are represented in `schemas/datamodel.schema.json` and the scoped canonical type/prose updates; storage never holds raw nonce or raw assertion material.
- [ ] Meets the repo definition of done (adapter and mock tests, negative-space/concurrency tests, `cargo fmt --all`, `cargo clippy --workspace -- -D warnings`, and applicable `cargo nextest` tests; record but do not fix the known unrelated three config-test baseline failures).
- [ ] Reviewable: a reviewer can run the shared conformance suite against each store and observe that exactly one concurrent writer or consumer wins.
