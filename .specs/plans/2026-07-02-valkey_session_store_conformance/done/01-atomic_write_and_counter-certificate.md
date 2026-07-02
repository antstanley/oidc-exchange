# Done Certificate — Task 01: atomic session write with counter increment and TTL rejection

**Task:** [01-atomic_write_and_counter.md](01-atomic_write_and_counter.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

> This certificate is a verification protocol for Task 01. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 01) ≡ every obligation O1…O5 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** `store_refresh_token` applies the session hash, its TTL, the user-set
  membership, an `EXPIRE … GT` set-TTL bump, and an `INCR {prefix}active_sessions` in one `fred`
  pipeline; a session whose `expires_at` is not in the future is rejected with `Error::StoreError`
  and writes no key; the `#[ignore]`-gated Valkey integration-test harness exists.
- **P2 — Obligations.** The task is done iff O1…O5 all hold. One Oi per definition-of-done item,
  in DoD order; O5 is the `Reviewable:` item.
- **P3 — Invariants.** Must not break `get_session_by_refresh_token`
  (`crates/adapters/src/valkey/mod.rs:84-151`), which reads the hash fields this task still writes,
  nor change the `Session` field set written into the hash (lines 40-49).

## Obligations

- **O1 — Atomic pipelined write with TTL rejection.**
  - *Claim:* `store_refresh_token` issues HSET, EXPIRE, SADD, the `EXPIRE … GT` set-TTL bump, and
    `INCR {prefix}active_sessions` through a single `fred` pipeline, and returns `Error::StoreError`
    for a non-future `expires_at` before writing anything.
  - *Evidence to collect:* read `crates/adapters/src/valkey/mod.rs` `store_refresh_token`; confirm
    a `client.pipeline()` batches the five commands and is executed once (not five separate awaited
    calls), and that the `ttl_seconds` guard returns `StoreError` before the pipeline is built.
    Run the store-path integration test (`--ignored`) and confirm the hash, the user-set member,
    the set TTL, and the counter at 1 all result from one store.
  - *Checks:* resolve the set-TTL option used in the `EXPIRE … GT` bump — confirm it is `fred`'s
    `ExpireOptions::GT` (only-extend), not the `None` third argument the current `expire` call
    passes at line 62. Resolve `Error::StoreError` to `crates/core/src/error.rs:36`.
  - *Status:* ✅ SATISFIED — `crates/adapters/src/valkey/mod.rs:48-134`: the `ttl_seconds` guard
    (lines 52-59) returns `Error::StoreError` before the pipeline is built (line 86); one
    `self.client.pipeline()` batches HSET (:88), EXPIRE key (:94), SADD (:100), `EXPIRE … NX`
    (:111), `EXPIRE … GT` (:117), and INCR (:123), executed once via `pipeline.all::<()>()`
    (:129) — not separately awaited commands (the per-command awaits queue into the fred 10
    `Pipeline`). The GT bump resolves to `fred::types::ExpireOptions::GT` (import at :4), not
    the old `None` third argument; `Error::StoreError` resolves to the
    `StoreError { detail }` variant in `crates/core/src/error.rs` (§ infrastructure errors).
    Live store test `store_refresh_token_writes_ttld_hash_set_member_and_counter` PASSED:
    one store yields a TTL'd hash (0 < TTL ≤ 3600), the user-set member, a bumped set TTL,
    and counter = 1. Note: the pipeline carries six commands, not five — an added
    `ExpireOptions::NX` bootstrap before the GT bump (GT treats a no-expiry set as infinite,
    so GT alone would never TTL a brand-new set); a justified superset of the claim.

- **O2 — Negative-space: zero/negative TTL rejected, no key; GT never shortens.**
  - *Claim:* a session with `expires_at` at or before now leaves no `{prefix}session:*` key and no
    counter increment; a later shorter-TTL store for the same user does not shorten the user-set TTL.
  - *Evidence to collect:* run the negative-TTL integration test — expect `store_refresh_token` to
    return `StoreError` and a follow-up `EXISTS {prefix}session:{hash}` / counter GET to show
    nothing was written. Run the GT-only-extend test — store with a long TTL, then a shorter TTL,
    and assert the set TTL did not decrease.
  - *Checks:* trace one concrete input — `expires_at = now` → `ttl_seconds = 0` → guard returns
    `StoreError` before any pipeline command → no hash, no INCR.
  - *Status:* ✅ SATISFIED — live test `store_refresh_token_rejects_non_future_expiry` PASSED:
    `expires_at = now` and `expires_at = now − 60s` both return `Error::StoreError`; a
    follow-up `EXISTS {prefix}session:{hash}` is false and the counter key does not exist.
    Live test `store_refresh_token_set_ttl_only_extends` PASSED: after a 3600s store, a 5s
    store for the same user leaves the set TTL > 5 (and ≤ the prior TTL). Trace confirmed:
    `expires_at = now` → `ttl_seconds = 0 < SESSION_TTL_SECONDS_MIN (1)` → `return Err` at
    mod.rs:54, before the pipeline at :86 — no hash, no INCR.

- **O3 — Named TTL floor, `active_sessions_key` helper, ≥2 assertions.**
  - *Claim:* the TTL floor is a named constant with units, the `active_sessions_key` helper exists,
    and `store_refresh_token` carries ≥2 meaningful assertions.
  - *Evidence to collect:* grep `store_refresh_token` and the module for numeric TTL literals —
    confirm the floor (e.g. `SESSION_TTL_SECONDS_MIN`) is a named `const` referenced by name; read
    the key helpers (lines 25-31) and confirm an `active_sessions_key` returning
    `{prefix}active_sessions` sits beside them; count the `assert!`/`assert_eq!`/`debug_assert!`
    calls in `store_refresh_token` (≥2, none `assert!(true)`).
  - *Status:* ✅ SATISFIED — `const SESSION_TTL_SECONDS_MIN: i64 = 1` at mod.rs:14, units in
    name and doc comment, referenced by name at :53 and :60; no bare numeric TTL literal
    remains in the function. `active_sessions_key` at mod.rs:40-42 returns
    `{prefix}active_sessions`, beside the other key helpers. Three meaningful assertions:
    `debug_assert!(ttl_seconds >= SESSION_TTL_SECONDS_MIN)` (:60),
    `assert!(!key.is_empty(), …)` (:66), `assert!(!user_sessions_key.is_empty(), …)` (:67-70).

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy/fmt clean, limits named.
  - *Evidence to collect:* run the repo's Rust gates from
    [development-guidelines.md](../../../development-guidelines.md) §"Definition of done" —
    `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run
    --workspace` (all clean); and the `#[ignore]` Valkey integration tests against a local server —
    expect PASS.
  - *Status:* ✅ SATISFIED — `cargo fmt --all --check` clean; `cargo clippy --workspace --
    -D warnings` clean; `cargo nextest run --workspace` → 215 passed, 5 skipped, 0 failed;
    the three `#[ignore]` Valkey tests PASSED against a local `valkey/valkey:8-alpine`.

- **O5 — Reviewable: store test shows TTL'd hash + member + counter at 1, GT green, negative-TTL rejected (Reviewable).**
  - *Claim:* a reviewer runs the Valkey `--ignored` tests against a local server and sees the store
    test writing a TTL'd hash + user-set member + counter at 1, the GT-only-extend test green, and
    the negative-TTL test asserting no key was created.
  - *Evidence to collect:* with a local Valkey running, run
    `cargo nextest run -p oidc-exchange-adapters -- --ignored valkey` — observe the three named
    assertions in the store, GT, and negative-TTL tests all pass.
  - *Status:* ✅ SATISFIED — exercised with a local Valkey (docker `valkey/valkey:8-alpine`,
    port 6379) via the nextest form of the DoD command,
    `cargo nextest run -p oidc-exchange-adapters --run-ignored=all -E 'test(valkey)'` →
    3 tests run: 3 passed (`store_refresh_token_writes_ttld_hash_set_member_and_counter`,
    `store_refresh_token_set_ttl_only_extends`,
    `store_refresh_token_rejects_non_future_expiry`) — TTL'd hash + user-set member +
    counter at 1, GT only-extend green, negative-TTL rejection with no key created.

## Regression check

- `crates/adapters/src/valkey/mod.rs:84` `get_session_by_refresh_token` reads the hash fields
  `store_refresh_token` writes: after a store, expect it to still return the full `Session` with
  every field round-tripped : ✅ PRESERVED — the diff leaves `get_session_by_refresh_token`
  (now mod.rs:137-204) untouched; the `fields` vec written into the hash (mod.rs:72-81) is
  byte-identical to the pre-change field set (same 8 fields, same RFC 3339 serialization,
  same `unwrap_or_default()` for optionals) and `session_key` is unchanged, so a stored
  session still round-trips into a full `Session`.

## Residue

- Outside the DoD: the change spec's `count_active_sessions`/revoke decrement paths are Task 02;
  cleanup reconciliation is Task 03. This certificate covers only the write path and the harness.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O5 all SATISFIED with evidence in hand — pipeline + guard verified by reading
mod.rs:48-134 and by all three `#[ignore]` Valkey tests passing against a live
valkey/valkey:8-alpine, fmt/clippy/nextest workspace gates all clean (215 passed) — and the
`get_session_by_refresh_token` regression surface is PRESERVED; the only deviation from the
authored protocol is a benign superset (an extra `EXPIRE … NX` bootstrap ahead of the GT
bump, needed because GT never sets a TTL on a set with no expiry).
