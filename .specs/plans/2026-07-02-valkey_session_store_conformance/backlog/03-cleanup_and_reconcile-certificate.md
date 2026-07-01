# Done Certificate — Task 03: cleanup — prune index sets and reconcile the counter

**Task:** [03-cleanup_and_reconcile.md](03-cleanup_and_reconcile.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified   <!-- validator sets: Validated YYYY-MM-DD -->

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
  - *Status:* ☐ unverified

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
  - *Status:* ☐ unverified

- **O3 — Named scan-batch bound, ≥2 assertions.**
  - *Claim:* the scan-batch bound is a named constant with units and `cleanup_expired_sessions`
    carries ≥2 meaningful assertions.
  - *Evidence to collect:* grep the function for numeric literals — confirm the SCAN `COUNT` hint is
    a named `const` (e.g. `SCAN_BATCH_COUNT`) referenced by name, not a magic number; count the
    `assert!`/`assert_eq!`/`debug_assert!` calls (≥2, e.g. the reconciled-counter-equals-live-count
    postcondition, none `assert!(true)`).
  - *Status:* ☐ unverified

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy/fmt clean, limits named.
  - *Evidence to collect:* run the repo's Rust gates from
    [development-guidelines.md](../../../development-guidelines.md) §"Definition of done" —
    `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run
    --workspace` (all clean); and the `#[ignore]` Valkey integration tests against a local server —
    expect PASS.
  - *Status:* ☐ unverified

- **O5 — Reviewable: 1s-TTL session expires, cleanup returns pruned count, set gone, count equals live keys (Reviewable).**
  - *Claim:* a reviewer sees a 1s-TTL session expire, cleanup return the pruned-member count, the
    emptied set gone, and `count_active_sessions` equal to the remaining live-key count.
  - *Evidence to collect:* with a local Valkey running, run the Valkey `--ignored` cleanup test and
    observe the returned count, the absent set, and the reconciled counter.
  - *Status:* ☐ unverified

## Regression check

- `crates/adapters/src/valkey/mod.rs:185` `count_active_sessions` reads the same
  `{prefix}active_sessions` key this task `SET`s during reconciliation: trace a count after a
  cleanup pass → expect it to return the reconciled live-key value, not stale drift : ☐ (PRESERVED / REGRESSION)

## Residue

- Outside the DoD: `revoke_all_user_sessions` does not opportunistically prune dead members
  (change spec Decision "Cleanup-only pruning") — periodic cleanup handles them; not an obligation here.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
