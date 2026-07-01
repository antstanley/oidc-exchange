# Done Certificate — Task 02: counter read and decrement on the revoke paths

**Task:** [02-counter_read_and_decrement.md](02-counter_read_and_decrement.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified   <!-- validator sets: Validated YYYY-MM-DD -->

> This certificate is a verification protocol for Task 02. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 02) ≡ every obligation O1…O5 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** `count_active_sessions` returns the `{prefix}active_sessions` counter (missing
  key → 0) instead of `DBSIZE`; `revoke_session` `DECR`s only when its `DEL` removed the key;
  `revoke_all_user_sessions` `DECR`s by the number of session keys actually deleted.
- **P2 — Obligations.** The task is done iff O1…O5 all hold. One Oi per definition-of-done item,
  in DoD order; O5 is the `Reviewable:` item.
- **P3 — Invariants.** Depends on the `{prefix}active_sessions` counter and `active_sessions_key`
  helper introduced by Task 01; must not break the existing `SREM`/user-set cleanup in
  `revoke_session` (lines 173-180) or the wholesale set delete in `revoke_all_user_sessions`
  (lines 227-232).

## Obligations

- **O1 — Counter read and gated decrements.**
  - *Claim:* `count_active_sessions` reads `{prefix}active_sessions` (missing → 0); `revoke_session`
    decrements only on an actual delete; `revoke_all_user_sessions` decrements by the number of keys
    actually deleted.
  - *Evidence to collect:* read `count_active_sessions` (`crates/adapters/src/valkey/mod.rs:185-196`)
    — confirm the `dbsize()` call is replaced by a `GET {prefix}active_sessions` with a
    missing-key-to-0 fallback; read `revoke_session` (154-183) — confirm the `DEL` return count
    gates the `DECR`; read `revoke_all_user_sessions` (205-235) — confirm the counter is decremented
    by the count of `DEL`s that removed a key.
  - *Checks:* resolve the `DEL` return handling in `revoke_session` — confirm the code branches on
    the delete count (`== 1`) rather than always decrementing. Resolve `active_sessions_key` to the
    helper added in Task 01.
  - *Status:* ☐ unverified

- **O2 — Negative-space: no double-decrement, missing counter reads 0.**
  - *Claim:* a repeated/already-expired `revoke_session` does not decrement a second time, and
    `count_active_sessions` on a prefix with no counter key returns 0 rather than erroring.
  - *Evidence to collect:* run the double-revoke integration test — store, revoke (count N→N−1),
    revoke the same token again, assert the counter is unchanged on the second call. Run the
    empty-prefix count test — expect `count_active_sessions` to return `Ok(0)`.
  - *Checks:* trace one concrete input — second `revoke_session(hash)` on an already-deleted key →
    `DEL` returns 0 → `DECR` is skipped → counter unchanged.
  - *Status:* ☐ unverified

- **O3 — ≥2 assertions per touched function, no magic numbers.**
  - *Claim:* each touched function carries ≥2 meaningful assertions and any parse/limit is a named
    constant.
  - *Evidence to collect:* count the `assert!`/`assert_eq!`/`debug_assert!` calls in
    `count_active_sessions`, `revoke_session`, and `revoke_all_user_sessions` (≥2 each, none
    `assert!(true)`); grep the three functions for numeric literals and confirm none is an
    unnamed magic number.
  - *Status:* ☐ unverified

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy/fmt clean, limits named.
  - *Evidence to collect:* run the repo's Rust gates from
    [development-guidelines.md](../../../development-guidelines.md) §"Definition of done" —
    `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run
    --workspace` (all clean); and the `#[ignore]` Valkey integration tests against a local server —
    expect PASS.
  - *Status:* ☐ unverified

- **O5 — Reviewable: counter tracks store→revoke→revoke-all, double-revoke leaves count unchanged (Reviewable).**
  - *Claim:* a reviewer sees the counter-lifecycle test drive `count_active_sessions` to the exact
    live-key count through store→revoke→revoke-all, and the double-revoke case leaving the count
    unchanged on the second call.
  - *Evidence to collect:* with a local Valkey running, run the Valkey `--ignored` counter-lifecycle
    test and observe the count reaching the exact live-key value after each operation and holding on
    the repeated revoke.
  - *Status:* ☐ unverified

## Regression check

- `crates/adapters/src/valkey/mod.rs:173` `revoke_session`'s `SREM` from the user set must still
  fire after a successful delete: trace a revoke of a live session → expect the user-set member
  removed as before, now plus a single counter `DECR` : ☐ (PRESERVED / REGRESSION)

## Residue

- Outside the DoD: natural TTL expiry still drifts the counter upward between cleanups (by design);
  reconciliation is Task 03, not an obligation here.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
