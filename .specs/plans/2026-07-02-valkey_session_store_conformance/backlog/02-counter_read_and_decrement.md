# Task 02 — Counter read and decrement on the revoke paths

**Plan:** [plan.md](../plan.md) · **Certificate:** [02-counter_read_and_decrement-certificate.md](02-counter_read_and_decrement-certificate.md)

**Implements:** [08-persistence.md](../../../service/specs/08-persistence.md) §"Session-only stores" (`count_active_sessions` reads the counter, maintained by `INCR` on store and `DECR` on explicit revoke) · change spec Implementation notes 2, 3, 5
**Depends on:** 01
**Produces:** `count_active_sessions` returns the `{prefix}active_sessions` counter (missing key → 0) instead of `DBSIZE`; `revoke_session` `DECR`s the counter only when its `DEL` actually removed the session key; `revoke_all_user_sessions` `DECR`s the counter by the number of session keys it actually deleted.
**Pointers:** `crates/adapters/src/valkey/mod.rs:185-196` (`count_active_sessions`, currently `DBSIZE`), `:154-183` (`revoke_session`), `:205-235` (`revoke_all_user_sessions`), `active_sessions_key` helper from task 01.

## Steps

- [ ] Replace the `dbsize()` body of `count_active_sessions` with a `GET {prefix}active_sessions`, treating a missing/absent key as `0` and parsing the stored integer.
- [ ] In `revoke_session`, capture the `DEL {key}` return count and `DECR {prefix}active_sessions` only when it deleted the key (return count `== 1`), so a repeated or already-expired revoke does not double-decrement; keep the existing `SREM` from the user set.
- [ ] In `revoke_all_user_sessions`, delete the session keys, count how many `DEL`s actually removed a key, then `DECR`/`DECRBY` the counter by exactly that number before deleting the user set.
- [ ] Add ≥2 meaningful assertions to each touched function (e.g. in `revoke_session` assert the `DEL` count is `0` or `1`; in `revoke_all_user_sessions` assert the decremented amount is `<=` the number of member hashes read).
- [ ] Extend the integration tests (reusing task 01's harness): counter tracks store then revoke (N → N−1); a second revoke of the same token does not decrement again; `revoke_all_user_sessions` decrements by the live-key count and leaves the counter consistent; `count_active_sessions` on an untouched prefix returns 0.

## Definition of done

- [ ] `count_active_sessions` reads `{prefix}active_sessions` (missing → 0); `revoke_session` decrements only on an actual delete; `revoke_all_user_sessions` decrements by the number of keys actually deleted.
- [ ] Negative-space test: a repeated/already-expired `revoke_session` does not double-decrement the counter, and a `count_active_sessions` against a prefix with no counter key returns 0 rather than erroring.
- [ ] Each touched function carries ≥2 meaningful assertions; no magic numbers (any parse/limit is a named constant).
- [ ] Meets the repo definition of done (tests, `cargo fmt`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace`; `#[ignore]` integration tests pass against a local Valkey — see plan.md baseline).
- [ ] Reviewable: with a local Valkey running, the counter-lifecycle integration test shows store→revoke→revoke-all driving `count_active_sessions` to the exact live-key count, and the double-revoke case leaving the count unchanged on the second call.

## Open questions

- Whether to use `DECRBY n` or a loop of `DECR` in `revoke_all_user_sessions`; `DECRBY` is the single-command choice and avoids an unbounded loop. Resolve at build time.
