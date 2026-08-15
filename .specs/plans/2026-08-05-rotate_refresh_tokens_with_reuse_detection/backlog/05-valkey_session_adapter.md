# Task 05 — Valkey session adapter

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted (done certificates are forbidden)

**Implements:** source spec §Session-only stores (Valkey), SR1–SR5, and active-counter clamp.
**Depends on:** 01 · domain_config_port_contract; 02 · shared_session_contract_harness
**Produces:** TTL-bound retired/family structures, conditional Lua rotation, full family revocation, safe counter reconciliation, and conformance coverage.
**Pointers:** `crates/adapters/src/valkey/mod.rs` and its unit/integration tests.

## Steps

- [ ] Add retirement-hash and family-set key constructors with expiration bounded by retention/family expiry.
- [ ] Implement `resolve_refresh_token`, `revoke_family`, and one `EVAL` Lua rotation conditioned on live existence; preserve unconditional pipeline writes for generation-0 storage.
- [ ] Make every counter decrement path—including family revocation and Lua rotation—clamp a negative observed counter to zero and emit one structured warning rather than asserting/panicking.
- [ ] Reconcile cleanup with retired/family structures and invoke shared SR1–SR5 coverage.

## Definition of done

- [ ] The script changes live, retired, family membership, user membership, TTLs, and counter as one conditional operation; a lost race returns false with no partial swap.
- [ ] Expiry is never omitted/extended past its family bound for retirement state.
- [ ] Seeding counter zero then revoking a live session/family returns `Ok`, leaves zero, and records `counter_clamped = true`; no unauthenticated revoke can panic from drift.
- [ ] Shared suite plus adapter-specific script, TTL, and counter negative tests pass.
- [ ] Done certificates remain intentionally absent.
