# Done Certificate — Task 04: update the canonical spec pages to the shipped Valkey behavior

**Task:** [04-spec_pages.md](04-spec_pages.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified   <!-- validator sets: Validated YYYY-MM-DD -->

> This certificate is a verification protocol for Task 04. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 04) ≡ every obligation O1…O5 below holds, each backed by the evidence the obligation
names (a file location or a read-back comparison) — not by assertion.

## Premises

- **P1 — Goal.** The two canonical pages describe the merged Valkey adapter — atomic pipelined
  writes, the maintained `{prefix}active_sessions` counter, the `EXPIRE … GT` TTL'd index sets,
  TTL rejection, and cleanup prune-and-reconcile — matching the code shipped in Tasks 01–03.
- **P2 — Obligations.** The task is done iff O1…O5 all hold. One Oi per definition-of-done item,
  in DoD order; O5 is the `Reviewable:` item.
- **P3 — Invariants.** Documentation-only; must not alter any code or the
  `canonical-types.schema.json` (no type change in this change spec).

## Obligations

- **O1 — Replacement prose applied with bumped dates.**
  - *Claim:* both canonical pages carry the change spec's replacement prose for the Valkey adapter,
    with bumped `**Date:**` headers.
  - *Evidence to collect:* read `.specs/service/specs/08-persistence.md` §"Session-only stores" and
    confirm the Valkey bullet matches the change spec's `08-persistence.md → Session-only stores
    (Modify)` block; read `.specs/service/specs/02-ports-and-adapters.md` §"Adapter inventory" and
    confirm the `SessionRepository | Valkey/Redis` row matches the change spec's `02-ports-and-adapters.md
    → Adapter inventory (Modify)` row; confirm each page's `**Date:**` header is bumped.
  - *Status:* ☐ unverified

- **O2 — Every behavioral claim backed by shipped code.**
  - *Claim:* each behavioral claim in the updated prose (atomic pipeline, `GT` set-TTL bump,
    `INCR`/`DECR`/reconcile counter, TTL rejection, cleanup return value) is backed by code in
    `crates/adapters/src/valkey/mod.rs` from Tasks 01–03.
  - *Evidence to collect:* for each claim in the two updated sections, locate the backing code —
    the pipeline and `GT` bump and `INCR` in `store_refresh_token`, the counter `GET`/`DECR` in
    `count_active_sessions`/`revoke_session`/`revoke_all_user_sessions`, the SCAN-prune-and-`SET`
    reconcile in `cleanup_expired_sessions` — and confirm no claim lacks a code counterpart.
  - *Checks:* cross-check the counter drift claim — confirm the prose says the counter over-reports
    between cleanups (natural TTL expiry cannot decrement) and that the code has no decrement on the
    natural-expiry path, so prose and code agree.
  - *Status:* ☐ unverified

- **O3 — No type change; schema untouched.**
  - *Claim:* `canonical-types.schema.json` is untouched (documentation-only task).
  - *Evidence to collect:* confirm `.specs/service/specs/canonical-types.schema.json` (and any
    `schemas/` sidecar) is unchanged by this task's diff.
  - *Status:* ☐ unverified

- **O4 — Meets the repo definition of done for a docs change.**
  - *Claim:* the change description states the *why*; no code test applies.
  - *Evidence to collect:* confirm the change description explains why the prose changed (to match
    the shipped adapter); confirm the diff touches only the two `.specs/service/specs/*.md` pages,
    so the code test/lint gates in [development-guidelines.md](../../../development-guidelines.md)
    §"Definition of done" have no code surface to run against.
  - *Status:* ☐ unverified

- **O5 — Reviewable: each prose claim honored by the shipped code (Reviewable).**
  - *Claim:* a reviewer reads the two updated sections against `valkey/mod.rs` and finds each claim
    honored by the shipped code.
  - *Evidence to collect:* read the two updated sections side by side with
    `crates/adapters/src/valkey/mod.rs` and confirm every behavioral statement resolves to a code
    location — no divergence.
  - *Status:* ☐ unverified

## Regression check

- No existing code is modified — the task edits only `.specs/service/specs/08-persistence.md` and
  `.specs/service/specs/02-ports-and-adapters.md`. Nothing to regress; confirm the diff touches no
  `.rs` file : ☐ (PRESERVED / REGRESSION)

## Residue

- The change spec's "Merge plan" (flip Status to Merged, move to `changes/merged/`, update
  `.specs/README.md`) is handled centrally by the orchestrator, not by this task; not an obligation here.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
