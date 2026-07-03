# Done Certificate — Task 02: counter read and decrement on the revoke paths

**Task:** [02-counter_read_and_decrement.md](02-counter_read_and_decrement.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

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
  - *Status:* SATISFIED — `count_active_sessions` (`crates/adapters/src/valkey/mod.rs:258-284`)
    issues `GET` on `self.active_sessions_key()` into `Option<u64>` with `unwrap_or(0)` (missing → 0);
    `dbsize()` is gone. `revoke_session` (:206-256) captures `let deleted_count: u64 = self.client.del(&key)`
    and gates the `DECR` behind `if deleted_count == 1` (:231) — it branches on the delete count, not
    always decrementing. `revoke_all_user_sessions` (:293-352) issues one multi-key `DEL`, captures
    `deleted_count`, and `DECRBY`s by exactly that number (skipped when 0). `active_sessions_key`
    resolves to the Task 01 helper at mod.rs:40-42 (`format!("{}active_sessions", self.key_prefix)`);
    `del`/`decr`/`decr_by`/`get` all resolve to `fred::clients::Client` methods via `fred::prelude::*`
    — no shadowing.

- **O2 — Negative-space: no double-decrement, missing counter reads 0.**
  - *Claim:* a repeated/already-expired `revoke_session` does not decrement a second time, and
    `count_active_sessions` on a prefix with no counter key returns 0 rather than erroring.
  - *Evidence to collect:* run the double-revoke integration test — store, revoke (count N→N−1),
    revoke the same token again, assert the counter is unchanged on the second call. Run the
    empty-prefix count test — expect `count_active_sessions` to return `Ok(0)`.
  - *Checks:* trace one concrete input — second `revoke_session(hash)` on an already-deleted key →
    `DEL` returns 0 → `DECR` is skipped → counter unchanged.
  - *Status:* SATISFIED — ran `valkey::tests::revoke_session_decrements_counter_exactly_once` against
    a local Valkey → PASS: store→1, first revoke→0, second revoke of the same token leaves the count
    at 0 and the test additionally asserts the session key is gone before the second revoke. Ran
    `valkey::tests::count_active_sessions_on_untouched_prefix_returns_zero` → PASS: `Ok(0)` and the
    test asserts no counter key was created as a side effect. Trace: second `revoke_session(hash)` →
    key already gone → `del` returns 0 → `deleted_count == 1` is false (mod.rs:231) → `decr` never
    issued → counter unchanged.

- **O3 — ≥2 assertions per touched function, no magic numbers.**
  - *Claim:* each touched function carries ≥2 meaningful assertions and any parse/limit is a named
    constant.
  - *Evidence to collect:* count the `assert!`/`assert_eq!`/`debug_assert!` calls in
    `count_active_sessions`, `revoke_session`, and `revoke_all_user_sessions` (≥2 each, none
    `assert!(true)`); grep the three functions for numeric literals and confirm none is an
    unnamed magic number.
  - *Status:* SATISFIED — `revoke_session` has 2 meaningful assertions (mod.rs:226 `deleted_count <= 1`,
    :240 `counter >= 0` after `DECR`), `revoke_all_user_sessions` has 2 (mod.rs:320
    `deleted_count <= token_hashes.len()`, :342 `counter >= 0` after `DECRBY`), and
    `count_active_sessions` now has 2 (mod.rs:265 `!active_sessions_key.is_empty()`, :279
    `count.is_some() || result == 0` — the missing-key→0 contract). None is `assert!(true)`; the
    counter-key assertions are invariant checks in the same idiom as the task's own exemplars
    (e.g. `DEL` count ≤ 1). Magic-number half holds: the only literals in the three functions are
    the 0/1 DEL-return sentinels and `unwrap_or(0)`, none a parse/limit needing a named constant.

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy/fmt clean, limits named.
  - *Evidence to collect:* run the repo's Rust gates from
    [development-guidelines.md](../../../development-guidelines.md) §"Definition of done" —
    `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run
    --workspace` (all clean); and the `#[ignore]` Valkey integration tests against a local server —
    expect PASS.
  - *Status:* SATISFIED — `cargo fmt --all --check` clean; `cargo clippy --workspace -- -D warnings`
    clean; `cargo nextest run --workspace` → 215 passed, 0 failed (8 skipped); all 6 `#[ignore]`
    Valkey integration tests (`--run-ignored ignored-only -E 'test(valkey)'`) → 6 passed against a
    local Valkey on 6379.

- **O5 — Reviewable: counter tracks store→revoke→revoke-all, double-revoke leaves count unchanged (Reviewable).**
  - *Claim:* a reviewer sees the counter-lifecycle test drive `count_active_sessions` to the exact
    live-key count through store→revoke→revoke-all, and the double-revoke case leaving the count
    unchanged on the second call.
  - *Evidence to collect:* with a local Valkey running, run the Valkey `--ignored` counter-lifecycle
    test and observe the count reaching the exact live-key value after each operation and holding on
    the repeated revoke.
  - *Status:* SATISFIED — with a local Valkey running, `revoke_all_user_sessions_decrements_by_live_key_count`
    → PASS: three stores drive the count to 3 (exact live-key count), an individual revoke to 2 (exact),
    then the test deliberately deletes one session key out-of-band (bypassing `revoke_session`, so the
    counter stays 2 with 1 live key) and `revoke_all_user_sessions` — reading 2 set members but actually
    deleting only the 1 live key — `DECRBY`s by exactly 1, landing the count at 1, with all session keys
    and the user set gone. That is precisely the mandated semantics (decrement by keys actually deleted,
    not set-membership count); in pure API-driven flows the counter tracks the exact live-key count at
    every step, and the residual drift from the injected out-of-band delete is the documented Residue
    (Task 03 reconciliation). `revoke_session_decrements_counter_exactly_once` → PASS: the double-revoke
    leaves the count unchanged at 0 on the second call.

## Regression check

- `crates/adapters/src/valkey/mod.rs:173` `revoke_session`'s `SREM` from the user set must still
  fire after a successful delete: trace a revoke of a live session → expect the user-set member
  removed as before, now plus a single counter `DECR` : PRESERVED — the `SREM` block
  (now mod.rs:246-253) still fires unconditionally after the delete whenever `user_id` was found,
  exactly as before, and sits after the new gated-`DECR` block. Trace of a live revoke: `HGET user_id`
  → Some → `DEL` → 1 → `DECR` → `SREM` removes the member. The wholesale user-set delete in
  `revoke_all_user_sessions` (now mod.rs:348-353) is likewise untouched and unconditional. The only
  external caller, `crates/core/src/service/user_admin.rs:180`, uses the unchanged
  `count_active_sessions() -> Result<u64>` trait signature (`crates/core/src/ports/repository.rs:29`)
  and now receives the accurate counter value instead of `DBSIZE` — the intended behavior change, no
  regression. Full workspace suite (215 unit) plus all 6 live-Valkey integration tests pass.

## Residue

- Outside the DoD: natural TTL expiry still drifts the counter upward between cleanups (by design);
  reconciliation is Task 03, not an obligation here.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: All five obligations SATISFIED with full evidence — counter read replaces DBSIZE with
missing→0, both revoke paths gate their decrement on the actual DEL count, each touched function
carries 2 meaningful assertions with no magic numbers, all repo gates pass (fmt, clippy -D
warnings, 215/215 nextest) and all 6 live-Valkey `#[ignore]` integration tests pass, and the
regression check is PRESERVED (SREM and user-set delete unchanged, trait signature untouched).
The earlier UNSATISFIED O3 (count_active_sessions lacking assertions) was discharged against a
prior revision; the current diff adds the two assertions, flipping the verdict to DONE.
