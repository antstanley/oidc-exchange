# Task 04 — Update the canonical spec pages to the shipped Valkey behavior

**Plan:** [plan.md](../plan.md) · **Certificate:** [04-spec_pages-certificate.md](04-spec_pages-certificate.md)

**Implements:** [08-persistence.md](../../../service/specs/08-persistence.md) §"Session-only stores" and [02-ports-and-adapters.md](../../../service/specs/02-ports-and-adapters.md) §"Adapter inventory" (change spec §"Proposed changes")
**Depends on:** 01, 02, 03
**Produces:** The two canonical pages describe the merged Valkey adapter — atomic pipelined writes, the maintained `{prefix}active_sessions` counter, the `EXPIRE … GT` TTL'd index sets, TTL rejection, and the cleanup prune-and-reconcile — matching the code shipped in tasks 01–03.
**Pointers:** `.specs/service/specs/08-persistence.md` §"Session-only stores" (the Valkey bullet, currently the two-line description) and `.specs/service/specs/02-ports-and-adapters.md` §"Adapter inventory" (the `SessionRepository | Valkey/Redis` row); the exact replacement prose is quoted in the change spec's two "Proposed changes" blocks.

## Steps

- [ ] Replace the Valkey bullet under 08-persistence §"Session-only stores" with the change spec's `08-persistence.md → Session-only stores (Modify)` block (keys, atomic pipeline, `EXPIRE … GT`, maintained counter with drift, cleanup prune-and-reconcile).
- [ ] Replace the `SessionRepository | Valkey/Redis` row in 02-ports-and-adapters §"Adapter inventory" with the change spec's `02-ports-and-adapters.md → Adapter inventory (Modify)` row.
- [ ] Bump each page's `**Date:**` header to the merge date.
- [ ] Read back both pages and confirm the prose matches the code actually shipped in tasks 01–03 (no claim the implementation does not honor).

## Definition of done

- [ ] Both canonical pages carry the change spec's replacement prose for the Valkey adapter, with bumped `**Date:**` headers.
- [ ] Consistency check: every behavioral claim in the updated prose (atomic pipeline, `GT` set-TTL bump, `INCR`/`DECR`/reconcile counter, TTL rejection, cleanup return value) is backed by code in `crates/adapters/src/valkey/mod.rs` from tasks 01–03; no divergence.
- [ ] No type changes, so `canonical-types.schema.json` is untouched (documentation-only task; the repo DoD's test/lint gates apply to code, none of which changes here).
- [ ] Meets the repo definition of done for a docs change (the change description states the *why*; no code test applies — see plan.md baseline).
- [ ] Reviewable: a reviewer reads the two updated sections against `valkey/mod.rs` and finds each claim honored by the shipped code.

## Open questions

- (None at this stage.)
