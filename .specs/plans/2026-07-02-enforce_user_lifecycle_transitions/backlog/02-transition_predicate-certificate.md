# Done Certificate — Task 02: transition predicate

**Task:** [02-transition_predicate.md](02-transition_predicate.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified

> This certificate is a verification protocol for Task 02. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 02) ≡ every obligation O1…O5 below holds, each backed by the evidence the obligation
names — not by assertion.

## Premises

- **P1 — Goal.** The task produces `UserStatus::can_transition_to`, a pure predicate encoding the lifecycle, with a full ordered-pair truth-table test.
- **P2 — Obligations.** Done iff O1…O5 all hold. One Oi per definition-of-done item, in DoD order; O5 is the Reviewable item.
- **P3 — Invariants.** Must not change the `UserStatus` variants or its `serde` representation (`crates/core/src/domain/user.rs:27-35`); the predicate is additive.

## Obligations

- **O1 — `can_transition_to` returns the correct verdict for all nine ordered pairs.**
  - *Claim:* for every `(current, target)` in `{Active, Suspended, Deleted}²`, the predicate matches the lifecycle: `Active→Active` true, `Active→Suspended` true, `Active→Deleted` true, `Suspended→Active` true, `Suspended→Suspended` true, `Suspended→Deleted` true, `Deleted→*` false.
  - *Evidence to collect:* read `can_transition_to` in `crates/core/src/domain/user.rs`; run the unit test enumerating the full 9-cell matrix — expect PASS with the truth values above.
  - *Status:* ☐ unverified

- **O2 — Negative-space: `Deleted → {Active, Suspended, Deleted}` all false; `Suspended → Deleted` true.**
  - *Claim:* no status patch leaves `Deleted`, and a suspended user is deletable.
  - *Evidence to collect:* run the test cases asserting `Deleted.can_transition_to(&Active) == false`, `== Suspended` false, `== Deleted` false, and `Suspended.can_transition_to(&Deleted) == true` — expect PASS.
  - *Status:* ☐ unverified

- **O3 — The predicate is pure and carries at least two meaningful assertions across its tests.**
  - *Claim:* `can_transition_to` performs no I/O and does not mutate `self`; its tests assert both allowed and rejected pairs.
  - *Evidence to collect:* read the signature (`&self, &UserStatus -> bool`) and body — confirm no `await`, no repo/port call, no `&mut self`; count assertions in the test module (≥2 meaningful).
  - *Status:* ☐ unverified

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean.
  - *Evidence to collect:* run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` (per `.specs/development-guidelines.md` §Definition of done) — expect clean.
  - *Status:* ☐ unverified

- **O5 — Reviewable: every diagram edge maps to true, every off-diagram pair to false.**
  - *Claim:* a reviewer reading the truth-table test confirms each drawn edge is `true` and `Deleted` has no outgoing edge.
  - *Evidence to collect:* read the matrix test and cross-check against the [01-domain-model.md](../../../service/specs/01-domain-model.md) diagram; confirm the mapping is complete and `Deleted` rows are all `false`.
  - *Status:* ☐ unverified

## Regression check

- No existing caller yet — `can_transition_to` is newly added and consumed only by Task 03. No existing code in scope to regress : ☐ (PRESERVED / REGRESSION)

## Residue

- None noted at authoring.

## Conclusion

VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
