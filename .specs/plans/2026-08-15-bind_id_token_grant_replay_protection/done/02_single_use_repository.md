# Task 02 — Single-use repository

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted at user request

**Implements:** [`.specs/changes/2026-08-05-bind_id_token_grant_replay_protection.md` §Proposed changes → `02-ports-and-adapters.md` and `08-persistence.md`](../../../changes/2026-08-05-bind_id_token_grant_replay_protection.md), [§Type changes](../../../changes/2026-08-05-bind_id_token_grant_replay_protection.md), [§Implementation notes 2](../../../changes/2026-08-05-bind_id_token_grant_replay_protection.md); [01-domain-model.md §ID scheme and §Required query patterns](../../../service/specs/01-domain-model.md), [02-ports-and-adapters.md §SessionRepository](../../../service/specs/02-ports-and-adapters.md), [08-persistence.md §DynamoDB, §PostgreSQL, §SQLite, and §Session-only stores](../../../service/specs/08-persistence.md)
**Depends on:** —
**Produces:** Every session repository atomically claims or consumes a digest-keyed, expiry-aware single-use record, including the in-memory test implementation.
**Pointers:** `crates/core/src/ports/repository.rs:24-34`, `crates/adapters/src/{dynamo,postgres,sqlite,lmdb,valkey}/`, `crates/test-utils/src/lib.rs:21-49`, `schemas/datamodel.schema.json`, `crates/adapters/src/lmdb/mod.rs:29`

## Steps

- [x] Add `put_single_use(key, expires_at)` and `take_single_use(key)` to `SessionRepository`, documenting atomic insert-if-absent/remove-and-report behavior and expired-is-absent semantics; expand cleanup semantics to include these records.
  - Both trait methods carry full doc contracts (exactly-one-of-N-wins, absent/burned/expired indistinguishable on take, namespaced-digest-only keys). `cleanup_expired_sessions`'s doc now states the sweep reclaims single-use records where the store lacks native expiry and that its count covers both kinds.
- [x] Add the logical `SingleUseRecord` representation and storage schema updates, keeping only namespaced digests and expiry in persistent state.
  - `crates/core/src/domain/single_use.rs` holds the logical type (`key` + `expires_at`, nothing else). Storage schemas folded: `schemas/datamodel.schema.json` gains the `SingleUseRecord` definition, `schemas/dynamodb/table-design.json` gains the `SINGLEUSE#` item schema plus `put_single_use`/`take_single_use` access patterns, and `.specs/service/specs/canonical-types.schema.json` gains the `SingleUseRecord` `$def`. Canonical prose folded into 01-domain-model (entity + query-pattern rows), 02-ports-and-adapters (§SessionRepository), and 08-persistence (new "Single-use records" section; LMDB bullet corrected to three named databases).
- [x] Implement adapter-native atomic semantics for DynamoDB, Postgres, SQLite, Valkey, and LMDB, including SQL DDL/index migration, Dynamo/Valkey native expiry, and one additional LMDB named database with the appropriately raised database limit.
  - Per the 08-persistence table: DynamoDB conditional `PutItem` (`attribute_not_exists(pk) OR expires_at < :now`) + `DeleteItem … ReturnValues=ALL_OLD` with read-back expiry re-validation and numeric `ttl`; Postgres/SQLite `INSERT … ON CONFLICT … DO UPDATE … WHERE expires_at < now()` + `DELETE … AND expires_at > now() RETURNING 1` with idempotent inline DDL (`single_use` table + `expires_at` index); Valkey `SET NX EX` behind a named TTL floor constant (`SINGLE_USE_TTL_SECONDS_MIN`) + `GETDEL`; LMDB one-write-txn claim/burn in a third named DB with `max_dbs(3)`.
- [x] Extend cleanup to reclaim expired single-use records where native expiry does not, while making both claim operations correct without a reaper.
  - Postgres, SQLite, LMDB, and `MockRepository` sweep both kinds and sum the counts; DynamoDB (TTL) and Valkey (`SET EX`) need no sweep by contract. The conformance suite exercises correctness without any sweep having run (expired-is-absent for both put and take).
- [x] Extend `MockRepository` with shared atomic single-use state and write cross-adapter conformance tests covering first claim, duplicate claim, consume, expired-is-absent, cleanup, and concurrent contention.
  - Single-use state lives under one `Arc<Mutex<…>>` shared by every clone, so clones race exactly as against a real store; `get_single_use_record` exposes stored records for test introspection. The shared scenarios live in `test_utils::single_use_conformance` and are wired into all five adapters plus the mock itself.

## Definition of done

- [x] `put_single_use` succeeds exactly once for one live key and treats an expired record as absent; `take_single_use` consumes a live record exactly once and never accepts an expired record.
  - Asserted by `first_claim_wins_duplicate_loses`, `consume_live_record_exactly_once` (including never-inserted key → false), and `expired_record_is_absent_to_put_and_take` (take refuses; put reclaims without a sweep).
- [x] Two concurrent claims of one key produce exactly one success for every adapter, and adapter failures remain typed `StoreError` values rather than silent fallback.
  - `concurrent_put_has_exactly_one_winner` / `concurrent_take_has_exactly_one_winner` run 8 racing tasks through a `JoinSet` per adapter; every error path maps to `Error::StoreError` (DynamoDB maps only `ConditionalCheckFailedException` to `Ok(false)`).
- [x] SQL/LMDB schema changes are idempotent and cleanup counts both sessions and single-use records without changing live-session behavior.
  - SQLite has an explicit repeated-migration idempotency test; Postgres DDL is `CREATE TABLE IF NOT EXISTS`/`CREATE INDEX IF NOT EXISTS` via the same idempotent block. The cleanup scenario asserts an exact count of one expired session plus one expired record while live records/sessions survive untouched.
- [x] New persistent fields are represented in `schemas/datamodel.schema.json` and the scoped canonical type/prose updates; storage never holds raw nonce or raw assertion material.
  - Completed in this task's follow-up commit (the implementation commit had landed code-only): datamodel definition, DynamoDB table-design entry, canonical `$def`, and the 01/02/08 prose folds listed above.
- [x] Meets the repo definition of done (adapter and mock tests, negative-space/concurrency tests, `cargo fmt --all`, `cargo clippy --workspace -- -D warnings`, and applicable `cargo nextest` tests; record but do not fix the known unrelated three config-test baseline failures).
  - Baseline correction carried over from task 01: the plan's "three failing config tests" note is stale — merged PR #36 fixed them. At the implementation commit the workspace ran green (418 passed / 44 skipped, fmt + clippy clean).
- [x] Reviewable: a reviewer can run the shared conformance suite against each store and observe that exactly one concurrent writer or consumer wins.
  - One suite (`test_utils::single_use_conformance`) is invoked per adapter from its own test module; LMDB and mock run unattended, DynamoDB Local / Postgres / SQLite / Valkey variants are `#[ignore]`d like their existing suites.

## Notes

- Key formats are fixed here for wave B: nonces are stored as `"nonce:<sha256hex>"`; assertion-replay markers as `"assertion:<provider>:<sha256hex(jti)>"`, or `"assertion:<provider>:d:<sha256hex(compact_jwt)>"` when the token carries no `jti` (the `d:` discriminator keeps a literal `jti` from colliding with a digest). Task 04 owns building those strings from verified claims.
- Wave-B contract: `put_single_use(key, exp)` returns `true` only when *this* call claimed the key — that boolean is the replay verdict; `take_single_use("nonce:<hash>")` is both the nonce check and its burn. Both treat expired as absent, so no reaper coordination is needed.
- Valkey rejects a `put` whose `expires_at` is not strictly in the future before creating any key (a born-dead record would need `SET EX 0`, which the server refuses); the other adapters accept writing an already-expired record because it reads as absent everywhere.
- `MockRepository::get_single_use_record(key)` exists for test introspection only (task 04/05 tests can assert what was stored); it is not part of the port.
