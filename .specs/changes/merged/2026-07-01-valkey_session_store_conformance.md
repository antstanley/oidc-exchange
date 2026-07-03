# Change: Bring the Valkey session store up to the SessionRepository contract

**Status:** Merged · **Date:** 2026-07-01 · **Merged:** 2026-07-03 · **Owner:** Ant Stanley · **Target:** crates/adapters

Fix three contract divergences in the Valkey `SessionRepository`: replace `DBSIZE` with a
maintained active-session counter, make the session write atomic and reject non-positive TTLs
so no immortal keys are created, and implement `cleanup_expired_sessions` — which also
reconciles the counter — so the `user_sessions:{user_id}` index sets (now TTL'd themselves)
stop growing without bound.

---

## Motivation

The Valkey adapter diverges from the port contract the other four session backends honor.
`count_active_sessions` returns `DBSIZE`, which counts every key in the database — including
the `user_sessions:*` index sets (roughly a 2x over-count) and any keys outside `key_prefix` —
where every other adapter counts sessions with `expires_at > now`. Health and admin metrics
built on the port are wrong on Valkey.

Two durability defects compound this. `store_refresh_token` issues HSET, EXPIRE, and SADD as
three non-atomic commands, and skips EXPIRE entirely when the computed TTL is `<= 0` — either
path can leave a TTL-less session hash that Valkey will never expire. And because
`cleanup_expired_sessions` is a hardcoded `Ok(0)` no-op, such keys are never reaped; likewise
the `user_sessions:{user_id}` set only shrinks on explicit revoke — natural TTL expiry never
removes members, and the set itself has no TTL, so long-lived users accumulate dead hashes
forever.

---

## Affected spec pages

| Canonical page                                                                               | Nature of change                                                                                 |
| -------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------ |
| [`.specs/service/specs/08-persistence.md`](../service/specs/08-persistence.md)               | Session-only stores: document Valkey's atomic writes, maintained session counter, TTL'd index sets, and cleanup reconciliation |
| [`.specs/service/specs/02-ports-and-adapters.md`](../service/specs/02-ports-and-adapters.md) | Adapter inventory: update the Valkey row's notes                                                 |

---

## Proposed changes

### `.specs/service/specs/08-persistence.md` → Session-only stores (Modify)

> - **Valkey/Redis (`adapters/valkey`)** — `fred` client; keys `{prefix}session:{hash}`, a
>   `{prefix}user_sessions:{user_id}` set, and a `{prefix}active_sessions` counter. A session
>   write applies the hash, its TTL, the user-set membership, an `INCR` of the counter, and a
>   bump of the user set's own TTL to the greatest member expiry — atomically (single
>   pipeline). The set-TTL bump uses `EXPIRE … GT` (only-extend), so a concurrent
>   shorter-lived write can never shorten the set's life, and idle users' index sets expire on
>   their own. A session whose `expires_at` is not in the future is rejected, so no TTL-less
>   key is ever created. `count_active_sessions` reads the counter, which is maintained by
>   `INCR` on store and `DECR` on explicit revoke; natural TTL expiry cannot decrement it, so
>   it drifts upward between cleanups. `cleanup_expired_sessions` prunes `user_sessions` set
>   members whose session key no longer exists, reconciles the counter by recomputing it from
>   a SCAN of live `{prefix}session:*` keys, and returns the number of members pruned; session
>   bodies themselves need no sweep.

### `.specs/service/specs/02-ports-and-adapters.md` → Adapter inventory (Modify)

> | SessionRepository | Valkey/Redis | `adapters/valkey` | `fred`; `{prefix}session:{hash}`, `{prefix}user_sessions:{user_id}` set (TTL bumped via `EXPIRE … GT`), `{prefix}active_sessions` counter; atomic pipelined writes; cleanup prunes index sets and reconciles the counter |

---

## Type changes

None.

---

## Implementation notes

1. `crates/adapters/src/valkey/mod.rs:37-81` — `store_refresh_token`: compute `ttl_seconds`
   first and return `Error::StoreError` when `<= 0`; issue HSET + EXPIRE + SADD +
   `EXPIRE {prefix}user_sessions:{user_id} ttl GT` + `INCR {prefix}active_sessions` through
   one `fred` pipeline (`client.pipeline()`) so a crash cannot leave a TTL-less hash, a
   half-written index, or a missed count. The `GT` option makes the set-TTL bump only-extend;
   it was added in Redis 7.0 and Valkey forked from Redis 7.2.4, so every Valkey release has
   it, and `fred` 10 exposes it via `ExpireOptions::GT`.
2. `crates/adapters/src/valkey/mod.rs:186-196` — `count_active_sessions`: replace `DBSIZE`
   with a GET of `{prefix}active_sessions` (missing key → 0).
3. `crates/adapters/src/valkey/mod.rs:154-184` — `revoke_session`: DECR the counter only when
   the session key was actually deleted (gate on DEL's return count so a repeated or
   already-expired revoke does not double-decrement).
4. `crates/adapters/src/valkey/mod.rs:199-203` — `cleanup_expired_sessions`: SCAN
   `{prefix}user_sessions:*`; for each set, SREM members whose `{prefix}session:{hash}` key
   fails EXISTS; delete emptied sets; then reconcile the counter — SCAN `{prefix}session:*`,
   count the live keys, and SET `{prefix}active_sessions` to that count; return total members
   removed.
5. `crates/adapters/src/valkey/mod.rs:206-235` — `revoke_all_user_sessions` deletes the set
   wholesale; additionally DECR the counter by the number of session keys actually deleted
   (DEL's return count). Reuse its key helpers in cleanup; no opportunistic EXISTS-pruning of
   dead members here — periodic cleanup handles them.
6. Extend the Valkey integration tests: counter tracks store/revoke/revoke-all,
   zero/negative TTL rejected, cleanup prunes a member whose session key was expired (use a
   1s TTL) and resets the drifted counter, and the user-set TTL only ever extends (a second
   shorter-lived write does not shorten it).

---

## Merge plan

1. Apply both `Proposed changes` blocks to their canonical pages; bump each page's `**Date:**`.
2. No schema change.
3. Flip `**Status:**` to `Merged`, stamp `**Merged:**`, move to `.specs/changes/merged/`.
4. Update `.specs/README.md`.

---

## Assumptions and open questions

### Assumptions

- The Valkey database may be shared (other keys outside `key_prefix` exist), so the counter
  key and all scans are prefix-scoped; SCAN with a MATCH pattern is acceptable on the periodic
  cleanup path at the session cardinalities the session-only topology targets.
- `cleanup_expired_sessions` is invoked periodically by the deployment (as
  [08-persistence.md](../service/specs/08-persistence.md) already assumes for DynamoDB-without-TTL);
  between runs the counter may over-report by the sessions that expired naturally since the
  last reconciliation.

### Decisions

- _Pipeline, not MULTI._ **A `fred` pipeline is sufficient** — the three commands are
  independent writes to two keys; transactional isolation buys nothing over one network
  round-trip here.
- _Cleanup counts index members, not sessions._ **Session bodies expire server-side (as on
  DynamoDB), so the port's "rows deleted" return reports dead index members pruned.**
- _Maintained counter, not SCAN._ **`count_active_sessions` reads a `{prefix}active_sessions`
  counter, INCR'd on store and DECR'd on explicit revoke.** Counting is O(1) at any session
  cardinality; the upward drift from natural TTL expiry (which cannot decrement) is
  reconciled by `cleanup_expired_sessions` recomputing the counter from live session keys.
- _TTL the `user_sessions` set._ **Each write bumps the set's own TTL to the greatest member
  expiry using `EXPIRE … GT`.** The only-extend semantics (Redis ≥ 7.0, hence every Valkey
  release) remove the race where a concurrent shorter-lived write could shorten the set's
  life, and idle users' index sets now expire without any cleanup pass.
- _Cleanup-only pruning._ **`revoke_all_user_sessions` does not opportunistically EXISTS-check
  members; periodic cleanup suffices for now.** Extra per-member round-trips on the revoke
  path buy little when staleness is already bounded by the cleanup interval and set TTLs.

### Open questions

- (None at this stage.)
