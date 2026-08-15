# 08 · Cursor-paginated bounded admin reads

**Status:** Backlog  
**Implements:** [source spec](../../../changes/2026-08-05-harden_admin_plane.md) §"Implementation notes" step 8; [02-ports-and-adapters](../../../service/specs/02-ports-and-adapters.md), [08-persistence](../../../service/specs/08-persistence.md), [01-domain-model](../../../service/specs/01-domain-model.md), [03-service-flows](../../../service/specs/03-service-flows.md), and [04-http-api](../../../service/specs/04-http-api.md) targets  
**Depends on:** 06 · generated internal API client; 07 · closed reserved-claim enforcement  
**Produces:** `GET /internal/users` returns bounded cursor `UserPage`s and adapter-specific bounded reads/counters/cache replace full-table admin work.

**Pointers:** `crates/core/src/ports/repository.rs`; `crates/core/src/service/user_admin.rs:319-341`; `crates/server/src/routes/internal.rs:47-61`; `crates/adapters/src/{dynamo,postgres,sqlite}/mod.rs`; `crates/test-utils/src/lib.rs`; core/server/adapter tests; generated schema/client from task 06.

## Work

- Change the repository/service/route signature to `list_users(cursor, limit) -> UserPage`; define `MAX_ADMIN_PAGE_SIZE = 200`, default 50, and clamp in core rather than handlers.
- Replace DynamoDB list materialization with one bounded scan per page and opaque adapter-issued cursor; implement `(created_at, id)` keyset cursors in PostgreSQL and SQLite. Preserve the source-specified DynamoDB short-page/non-null-cursor behaviour.
- Replace Dynamo user-status scans with transactional `STATS#USERS`/`COUNTS` maintenance on create/delete/status transitions, including conditional/concurrent write correctness; cache Dynamo active-session scans using configured bounded TTL.
- Update mocks, route/query parsing, schema/client, and full-stack tests. Measure Dynamo consumed read capacity rather than wall time; remove the resolved persistence open question during canonical merge work.

## Definition of done

- [ ] Absent cursor starts a page; `next_cursor = null` is the only completion signal; invalid/tampered cursor and limits below/at/above bounds have deterministic negative-path tests.
- [ ] Core always clamps to `MAX_ADMIN_PAGE_SIZE`; route tests cannot bypass it, and callers no longer accept or emit `offset`.
- [ ] Dynamo executes at most one bounded scan per list page and tests assert consumed capacity; PostgreSQL/SQLite keyset tests show no duplicates/skips across adjacent pages.
- [ ] Dynamo user counts remain correct across create/delete/status transitions under conditional failures; active-session cache obeys its named TTL and expiry behaviour.
- [ ] Generated console pagination follows `next_cursor` through a short page and terminates only at null.
- [ ] Rust and TypeScript format/lint/typecheck/relevant test suites pass; integration prerequisites (DynamoDB Local) are stated, and unrelated failures are recorded but not fixed.
- [ ] Reviewable: every admin read is bounded, cursor-compatible, and no longer materializes/scans unbounded user listings per request.
