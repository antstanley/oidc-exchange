# Task 05 — Valkey session adapter

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted (done certificates are forbidden)

**Implements:** source spec §Session-only stores (Valkey), SR1–SR5, and active-counter clamp.
**Depends on:** 01 · domain_config_port_contract; 02 · shared_session_contract_harness
**Produces:** TTL-bound retired/family structures, conditional Lua rotation, full family revocation, safe counter reconciliation, and conformance coverage.
**Pointers:** `crates/adapters/src/valkey/mod.rs` and its unit/integration tests.

## Steps

- [x] Add retirement-hash and family-set key constructors with expiration bounded by retention/family expiry.
- [x] Implement `resolve_refresh_token`, `revoke_family`, and one `EVAL` Lua rotation conditioned on live existence; preserve unconditional pipeline writes for generation-0 storage.
- [x] Make every counter decrement path—including family revocation and Lua rotation—clamp a negative observed counter to zero and emit one structured warning rather than asserting/panicking.
- [x] Reconcile cleanup with retired/family structures and invoke shared SR1–SR5 coverage.

## Definition of done

- [x] The script changes live, retired, family membership, user membership, TTLs, and counter as one conditional operation; a lost race returns false with no partial swap.
- [x] Expiry is never omitted/extended past its family bound for retirement state.
- [x] Seeding counter zero then revoking a live session/family returns `Ok`, leaves zero, and records `counter_clamped = true`; no unauthenticated revoke can panic from drift.
- [x] Shared suite plus adapter-specific script, TTL, and counter negative tests pass.
- [x] Done certificates remain intentionally absent.

## Completion notes

- Audited against the code on 2026-08-22 (wave B recovery): every criterion held as implemented; no gaps found (`crates/adapters/src/valkey/mod.rs`).
- Keys: `{prefix}retired:{hash}` hashes and `{prefix}family:{family_id}` sets join the existing session/user keys, all under the caller's prefix namespace. Retirement-record TTL is `min(now + refresh_reuse_retention, family expires_at)` via `RetiredRefreshToken::retention_deadline`, floored at `SESSION_TTL_SECONDS_MIN = 1` so a record is never written TTL-less.
- `ROTATION_SCRIPT` is one `EVAL`: it checks the live key's existence (CAS — returns `0`, nothing written), verifies ownership (error reply on user mismatch), refuses an existing replacement (`-1` → store error, caller bug), then in one atomic unit deletes the live row, moves user-set membership, installs the replacement hash + its TTL, writes the retirement record + TTL, and maintains family-set membership with `bump_ttl` greatest-of semantics so a concurrent shorter-lived write cannot shorten a set's life. Legacy rows (absent/empty `family_id`) install the replacement without any retirement record, matching tasks 03–04. The script deliberately does not touch the counter: the swap removes one live generation and installs exactly one, net zero by construction, and a counter comparison inside the script could only reintroduce the panic this task removes.
- Generation-0 storage keeps its unconditional pipeline (`store_refresh_token`), gated only by the pre-flight future-expiry check both paths share.
- Counter discipline: every decrement path goes through `decr_counter_clamped` — `DECRBY`, then on a negative observation `SET 0` plus exactly one structured `tracing::warn!` carrying `counter_clamped = true`, the observed value, and the amount. Wired into `revoke_session`, `revoke_family` (one clamped decrement for the whole family's live deletions, not per delete), `revoke_all_user_sessions`, and cleanup reconciliation. No decrement path asserts or unwinds, so drifted counters are unreachable from unauthenticated `POST /revoke`.
- Classification reads live hash → retired hash → successor liveness with the same expired-record-reads-`Unknown` rule as the other adapters.
- Cleanup scans its own prefix (batched at `SCAN_BATCH_COUNT = 256`), deletes dead session hashes and dead retired records, prunes family/user set members whose hashes died, resets the reconciled counter to the observed live count through the clamp helper, and reports honest counts (zero when nothing is dead).
- Coverage: `session_contract::assert_full_conformance(&store, "valkey-session-conformance")` plus adapter-local negative tests — legacy first-redemption swap and failed CAS, rotation-writes-retired-record-and-both-set-memberships, `retired_key_ttl_is_capped_at_family_expiry`, counter-drift clamps on both `revoke_session` and `revoke_family` (seeded below zero, revoked → `Ok`, key reads exactly 0), store-TTL monotonicity and non-future-expiry rejection, cleanup pruning of dead members. All normal-tier against a locally started Valkey via environment URL gating.
- Gates at completion: fmt clean, clippy `-D warnings` clean, nextest 432 passed / 43 skipped. No done certificate exists.
