# Task 06 — DynamoDB session adapter

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted (done certificates are forbidden)

**Implements:** source spec §DynamoDB persistence, SR1–SR5, consistent session/user reads, and authoritative roster.
**Depends on:** 01 · domain_config_port_contract; 02 · shared_session_contract_harness
**Produces:** strong classification, transactional swap and roster maintenance, complete family/user revocation, and ignored integration conformance coverage.
**Pointers:** `crates/adapters/src/dynamo/mod.rs`; DynamoDB tests and local integration setup.

## Steps

- [ ] Add `RETIRED#` items, family-oriented GSI sort keys, TTL, and serialisation for family/generation/retirement fields.
- [ ] Use `consistent_read(true)` for live/retired session classification and the refresh-path status read.
- [ ] Implement transactional rotation with conditional delete/puts and transactional maintenance of the user item’s authoritative live-session set/family map.
- [ ] Reimplement `revoke_family` and `revoke_all_user_sessions` from a strongly consistent user-item roster, preserving bounded retry/error behaviour for every delete.
- [ ] Invoke generic conformance under existing ignored/backend-gated tests; add explicit regression coverage that GSI staleness cannot report successful incomplete revocation.

## Definition of done

- [ ] Conditional transaction cancellation only maps to `false` for a moved live generation; all other failures remain store errors.
- [ ] Every successful session mutation leaves item storage and authoritative roster mutually consistent.
- [ ] Successful family/all-user revocation removes all named live and retirement entries rather than trusting eventual GSI enumeration.
- [ ] Dynamo shared SR1–SR5 tests retain ignored/environment gating but are runnable in the integration job.
- [ ] Done certificates remain intentionally absent.
