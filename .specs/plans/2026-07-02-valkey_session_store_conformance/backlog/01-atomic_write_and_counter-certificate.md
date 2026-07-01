# Done Certificate — Task 01: atomic session write with counter increment and TTL rejection

**Task:** [01-atomic_write_and_counter.md](01-atomic_write_and_counter.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified   <!-- validator sets: Validated YYYY-MM-DD -->

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
  - *Status:* ☐ unverified

- **O2 — Negative-space: zero/negative TTL rejected, no key; GT never shortens.**
  - *Claim:* a session with `expires_at` at or before now leaves no `{prefix}session:*` key and no
    counter increment; a later shorter-TTL store for the same user does not shorten the user-set TTL.
  - *Evidence to collect:* run the negative-TTL integration test — expect `store_refresh_token` to
    return `StoreError` and a follow-up `EXISTS {prefix}session:{hash}` / counter GET to show
    nothing was written. Run the GT-only-extend test — store with a long TTL, then a shorter TTL,
    and assert the set TTL did not decrease.
  - *Checks:* trace one concrete input — `expires_at = now` → `ttl_seconds = 0` → guard returns
    `StoreError` before any pipeline command → no hash, no INCR.
  - *Status:* ☐ unverified

- **O3 — Named TTL floor, `active_sessions_key` helper, ≥2 assertions.**
  - *Claim:* the TTL floor is a named constant with units, the `active_sessions_key` helper exists,
    and `store_refresh_token` carries ≥2 meaningful assertions.
  - *Evidence to collect:* grep `store_refresh_token` and the module for numeric TTL literals —
    confirm the floor (e.g. `SESSION_TTL_SECONDS_MIN`) is a named `const` referenced by name; read
    the key helpers (lines 25-31) and confirm an `active_sessions_key` returning
    `{prefix}active_sessions` sits beside them; count the `assert!`/`assert_eq!`/`debug_assert!`
    calls in `store_refresh_token` (≥2, none `assert!(true)`).
  - *Status:* ☐ unverified

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy/fmt clean, limits named.
  - *Evidence to collect:* run the repo's Rust gates from
    [development-guidelines.md](../../../development-guidelines.md) §"Definition of done" —
    `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run
    --workspace` (all clean); and the `#[ignore]` Valkey integration tests against a local server —
    expect PASS.
  - *Status:* ☐ unverified

- **O5 — Reviewable: store test shows TTL'd hash + member + counter at 1, GT green, negative-TTL rejected (Reviewable).**
  - *Claim:* a reviewer runs the Valkey `--ignored` tests against a local server and sees the store
    test writing a TTL'd hash + user-set member + counter at 1, the GT-only-extend test green, and
    the negative-TTL test asserting no key was created.
  - *Evidence to collect:* with a local Valkey running, run
    `cargo nextest run -p oidc-exchange-adapters -- --ignored valkey` — observe the three named
    assertions in the store, GT, and negative-TTL tests all pass.
  - *Status:* ☐ unverified

## Regression check

- `crates/adapters/src/valkey/mod.rs:84` `get_session_by_refresh_token` reads the hash fields
  `store_refresh_token` writes: after a store, expect it to still return the full `Session` with
  every field round-tripped : ☐ (PRESERVED / REGRESSION)

## Residue

- Outside the DoD: the change spec's `count_active_sessions`/revoke decrement paths are Task 02;
  cleanup reconciliation is Task 03. This certificate covers only the write path and the harness.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
