# Task 03 — Cleanup: prune index sets and reconcile the counter

**Plan:** [plan.md](../plan.md) · **Certificate:** [03-cleanup_and_reconcile-certificate.md](03-cleanup_and_reconcile-certificate.md)

**Implements:** [08-persistence.md](../../../service/specs/08-persistence.md) §"Session-only stores" (`cleanup_expired_sessions` prunes index members, reconciles the counter, returns members pruned) · change spec Implementation note 4
**Depends on:** 01, 02
**Produces:** `cleanup_expired_sessions` replaces the `Ok(0)` no-op with a real pass: it SCANs `{prefix}user_sessions:*`, SREMs members whose `{prefix}session:{hash}` key fails `EXISTS`, deletes emptied sets, reconciles `{prefix}active_sessions` by SCAN-counting live `{prefix}session:*` keys and `SET`ting the counter to that count, and returns the number of members removed.
**Pointers:** `crates/adapters/src/valkey/mod.rs:198-203` (`cleanup_expired_sessions`, currently `Ok(0)`), `session_key`/`user_sessions_key`/`active_sessions_key` helpers, `fred` `SCAN` with `MATCH`/`COUNT`.

## Steps

- [ ] Declare a named scan-batch constant with units (e.g. `SCAN_BATCH_COUNT: u32 = 256`) and use it as the `COUNT` hint for every `SCAN`; no magic number.
- [ ] SCAN `{prefix}user_sessions:*`; for each set, read its members and `SREM` every member whose `{prefix}session:{hash}` key fails `EXISTS`; after pruning, `DEL` the set if it is now empty. Accumulate the count of members removed.
- [ ] Reconcile the counter: SCAN `{prefix}session:*`, count the live keys, and `SET {prefix}active_sessions` to that count, clearing any upward drift from natural TTL expiry.
- [ ] Return the total members removed as `u64`.
- [ ] Add ≥2 meaningful assertions (e.g. assert the reconciled counter equals the counted live keys as a postcondition; assert the returned removed-count does not exceed the total members scanned).
- [ ] Extend the integration tests (reusing task 01's harness): store a session with a 1s TTL, wait for it to expire, run cleanup, and assert the stale `user_sessions` member was SREM'd, the emptied set deleted, the returned count reflects the pruned member, and `count_active_sessions` afterward equals the live-key count (drift reset).

## Definition of done

- [ ] `cleanup_expired_sessions` prunes dead `user_sessions` members, deletes emptied sets, reconciles the counter from a live-`{prefix}session:*` SCAN, and returns the members removed.
- [ ] Negative-space test: after a session's 1s TTL expires and its hash is gone, cleanup removes exactly its index member and resets a counter that had drifted above the live-key count; a cleanup over a prefix with no dead members returns 0 and leaves the counter equal to the live count.
- [ ] The scan-batch bound is a named constant with units; the function carries ≥2 meaningful assertions.
- [ ] Meets the repo definition of done (tests, `cargo fmt`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace`; `#[ignore]` integration tests pass against a local Valkey — see plan.md baseline).
- [ ] Reviewable: with a local Valkey running, the cleanup integration test shows a 1s-TTL session expiring, cleanup returning the pruned-member count, the emptied set gone, and `count_active_sessions` equal to the remaining live-key count.

## Open questions

- (None at this stage.)
