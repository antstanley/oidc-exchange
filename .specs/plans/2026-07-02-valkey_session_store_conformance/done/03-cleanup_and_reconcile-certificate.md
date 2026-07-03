# Done Certificate — Task 03: cleanup — prune index sets and reconcile the counter

**Task:** [03-cleanup_and_reconcile.md](03-cleanup_and_reconcile.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

> This certificate is a verification protocol for Task 03. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 03) ≡ every obligation O1…O5 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** `cleanup_expired_sessions` replaces the `Ok(0)` no-op with a pass that SCANs
  `{prefix}user_sessions:*`, SREMs members whose `{prefix}session:{hash}` key fails `EXISTS`,
  deletes emptied sets, reconciles `{prefix}active_sessions` from a SCAN of live
  `{prefix}session:*` keys, and returns the number of members removed.
- **P2 — Obligations.** The task is done iff O1…O5 all hold. One Oi per definition-of-done item,
  in DoD order; O5 is the `Reviewable:` item.
- **P3 — Invariants.** Depends on the counter, `active_sessions_key` (Task 01), and the maintained
  counter reads/decrements (Task 02); must not break `count_active_sessions`, which reads the
  counter this task `SET`s during reconciliation.

## Obligations

- **O1 — Prune, delete empty sets, reconcile, return members removed.**
  - *Claim:* `cleanup_expired_sessions` prunes dead `user_sessions` members, deletes emptied sets,
    reconciles the counter from a live-`{prefix}session:*` SCAN, and returns the members removed.
  - *Evidence to collect:* read `cleanup_expired_sessions`
    (`crates/adapters/src/valkey/mod.rs:198-203`) — confirm the `Ok(0)` no-op is replaced by: a
    SCAN of `{prefix}user_sessions:*`, an `EXISTS`-gated `SREM` per member, a `DEL` of emptied sets,
    a `SET {prefix}active_sessions` to the count from a `{prefix}session:*` SCAN, and a `u64` return
    of the members removed. Run the cleanup integration test and confirm the returned count matches
    the pruned members.
  - *Checks:* resolve the key patterns — confirm the SCAN `MATCH` patterns are prefix-scoped
    (`{prefix}user_sessions:*` and `{prefix}session:*`) via the existing key helpers, not an
    unscoped `*`. Resolve `active_sessions_key` to the Task 01 helper.
  - *Status:* ✅ SATISFIED — `cleanup_expired_sessions` (`crates/adapters/src/valkey/mod.rs:291-416`)
    replaces the `Ok(0)` no-op: `scan_buffered("{prefix}user_sessions:*", SCAN_BATCH_COUNT)` (mod.rs:299-307),
    per-member `EXISTS` gate via `self.session_key(member)` (mod.rs:324-336), batched `SREM` of dead
    members (mod.rs:340-352), `SCARD`-gated `DEL` of emptied sets (mod.rs:357-371), reconcile `SET`
    of `active_sessions_key` to the `{prefix}session:*` SCAN count (mod.rs:384-401), and
    `Ok(removed_members)` as `u64` (mod.rs:416). Checks: both SCAN patterns are built from
    `self.key_prefix` (mod.rs:299, 384) — string-identical to the `user_sessions_key`/`session_key`
    helper formats (mod.rs:37-42), not an unscoped `*`; `active_sessions_key` at mod.rs:395 resolves
    to the Task 01 inherent method `Self::active_sessions_key` (mod.rs:45-47) — no shadow. Cleanup
    integration test returned count 1 matching the one pruned member (test PASS, see O2/O5).

- **O2 — Negative-space: stale member pruned + drift reset; no-dead-member cleanup returns 0.**
  - *Claim:* after a 1s-TTL session's hash expires, cleanup removes exactly its index member and
    resets a counter that had drifted above the live-key count; a cleanup over a prefix with no dead
    members returns 0 and leaves the counter at the live count.
  - *Evidence to collect:* run the 1s-TTL cleanup integration test — store with a 1s TTL, wait for
    expiry, run cleanup, assert the stale member is SREM'd, the emptied set deleted, the return
    reflects the pruned member, and `count_active_sessions` equals the live-key count. Run the
    no-dead-member cleanup test — expect return `0` and an unchanged, correct counter.
  - *Checks:* trace one concrete input — session hash expired, member still in set → `EXISTS`
    returns 0 → `SREM` removes the member → set now empty → `DEL` set → reconcile `SET` counter to
    live count.
  - *Status:* ✅ SATISFIED — ran both `--ignored` tests against local Valkey (valkey/valkey:8-alpine):
    `cleanup_expired_sessions_prunes_only_member_deletes_set_and_resets_counter` PASS (2.59s) — stores
    a short-TTL session, waits past expiry, asserts hash gone, set still present with stale member,
    counter drifted at 1, then cleanup returns 1, set deleted, `count_active_sessions` == 0;
    `cleanup_expired_sessions_with_no_dead_members_returns_zero_and_matches_live_count` PASS —
    returns 0, live member untouched, counter == 1 == live count. Trace confirmed in code: `EXISTS`
    false (mod.rs:326-335) → member in `dead_members` → `SREM` (mod.rs:340) → `SCARD` == 0 →
    `DEL` (mod.rs:364-371) → `SET` counter to live SCAN count (mod.rs:396-401). Note: the test uses a
    2s TTL rather than the DoD's literal 1s, with an in-test justification (the 1s floor races
    `num_seconds()` truncation in `store_refresh_token`); the expiry-then-cleanup behaviour the DoD
    names is fully exercised.

- **O3 — Named scan-batch bound, ≥2 assertions.**
  - *Claim:* the scan-batch bound is a named constant with units and `cleanup_expired_sessions`
    carries ≥2 meaningful assertions.
  - *Evidence to collect:* grep the function for numeric literals — confirm the SCAN `COUNT` hint is
    a named `const` (e.g. `SCAN_BATCH_COUNT`) referenced by name, not a magic number; count the
    `assert!`/`assert_eq!`/`debug_assert!` calls (≥2, e.g. the reconciled-counter-equals-live-count
    postcondition, none `assert!(true)`).
  - *Status:* ✅ SATISFIED — `SCAN_BATCH_COUNT: u32 = 256` is a named `const` with units in its doc
    comment ("keys per page", mod.rs:16-19) and is the `COUNT` hint at both `scan_buffered` call
    sites (mod.rs:302, 387); no numeric literal appears in either SCAN call. Three meaningful
    assertions in the function: `assert!(srem_count <= dead_count)` (mod.rs:347),
    `assert!(removed_members <= members_scanned)` (mod.rs:374), and
    `assert_eq!(reconciled, live_count)` reconciled-counter postcondition (mod.rs:410) — ≥2, none
    trivially true.

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy/fmt clean, limits named.
  - *Evidence to collect:* run the repo's Rust gates from
    [development-guidelines.md](../../../development-guidelines.md) §"Definition of done" —
    `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run
    --workspace` (all clean); and the `#[ignore]` Valkey integration tests against a local server —
    expect PASS.
  - *Status:* ✅ SATISFIED — `cargo fmt --all --check` clean (no output); `cargo clippy --workspace
    -- -D warnings` clean; `cargo nextest run --workspace` → 215 passed, 0 failed, 10 skipped;
    `cargo nextest run -p oidc-exchange-adapters --run-ignored all -E 'test(valkey)'` against a
    local Valkey (docker `valkey/valkey:8-alpine` on 6379) → 8/8 PASS, including both new cleanup
    tests. Limits named (`SCAN_BATCH_COUNT`).

- **O5 — Reviewable: 1s-TTL session expires, cleanup returns pruned count, set gone, count equals live keys (Reviewable).**
  - *Claim:* a reviewer sees a 1s-TTL session expire, cleanup return the pruned-member count, the
    emptied set gone, and `count_active_sessions` equal to the remaining live-key count.
  - *Evidence to collect:* with a local Valkey running, run the Valkey `--ignored` cleanup test and
    observe the returned count, the absent set, and the reconciled counter.
  - *Status:* ✅ SATISFIED — exercised, not assumed: with the local Valkey container running, ran the
    `--ignored` test `cleanup_expired_sessions_prunes_only_member_deletes_set_and_resets_counter` →
    PASS in 2.59s. Its assertions observed: the short-TTL session hash expired (`EXISTS` false),
    cleanup returned 1 (the pruned member), the emptied `user_sessions` set is gone (`EXISTS` false
    after cleanup), and `count_active_sessions` == 0 == the remaining live-key count (drift from 1
    reset).

## Regression check

- `crates/adapters/src/valkey/mod.rs:185` `count_active_sessions` reads the same
  `{prefix}active_sessions` key this task `SET`s during reconciliation: trace a count after a
  cleanup pass → expect it to return the reconciled live-key value, not stale drift : ✅ PRESERVED —
  cleanup `SET`s the counter to `live_count` as `u64` (mod.rs:396-401); `count_active_sessions`
  (mod.rs:264-289) `GET`s the same `active_sessions_key()` as `Option<u64>` and the drift test asserts
  it returns 0 (the reconciled value) after cleanup, and 1 (live count) in the no-drift test. All
  pre-existing Valkey integration tests (store/revoke/count, Tasks 01-02) still pass 8/8, and the
  workspace suite is 215/215.

## Residue

- Outside the DoD: `revoke_all_user_sessions` does not opportunistically prune dead members
  (change spec Decision "Cleanup-only pruning") — periodic cleanup handles them; not an obligation here.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O5 all SATISFIED with direct evidence — the no-op is replaced by the prefix-scoped
prune/delete/reconcile pass returning removed members (O1), both negative-space cleanup tests pass
against a live Valkey (O2), `SCAN_BATCH_COUNT` and three meaningful assertions are in place (O3),
fmt/clippy/nextest and all 8 `--ignored` Valkey tests are clean (O4), the Reviewable flow was
exercised end-to-end (O5) — and the `count_active_sessions` regression surface is PRESERVED.
Minor drift noted, not a defect: the expiry test uses a 2s TTL instead of the DoD's literal 1s,
with an in-test justification (races against whole-second truncation at the 1s floor).
