# Done Certificate — Task 02: transition predicate

**Task:** [02-transition_predicate.md](02-transition_predicate.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-03

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
- **P3 — Invariants.** Must not change the `UserStatus` variants or its `serde` representation; the predicate is additive.

## Obligations

- **O1 — `can_transition_to` returns the correct verdict for all nine ordered pairs.**
  - *Claim:* for every `(current, target)` in `{Active, Suspended, Deleted}²`, the predicate matches the lifecycle: `Active→{Active,Suspended,Deleted}` true, `Suspended→{Active,Suspended,Deleted}` true, `Deleted→*` false.
  - *Evidence collected:* `can_transition_to` at `crates/core/src/domain/user.rs:52-64` is a total, wildcard-free match — `(Deleted, _) => false`, `(_, Deleted) => true`, `(Active|Suspended, Active|Suspended) => true` — which yields exactly the truth table above. The unit test `can_transition_to_matches_full_truth_table` enumerates all nine `(current, target, expected)` cells and PASSED in `cargo nextest run --workspace` (287/287).
  - *Status:* ☑ SATISFIED

- **O2 — Negative-space: `Deleted → {Active, Suspended, Deleted}` all false; `Suspended → Deleted` true.**
  - *Claim:* no status patch leaves `Deleted`, and a suspended user is deletable.
  - *Evidence collected:* `deleted_is_strictly_terminal` asserts `Deleted.can_transition_to(&Active|&Suspended|&Deleted) == false` and `Suspended.can_transition_to(&Deleted) == true`; PASSED in the workspace run. The `(Deleted, _) => false` arm precedes the `(_, Deleted) => true` arm, so `Deleted → Deleted` is correctly `false` (no self-loop).
  - *Status:* ☑ SATISFIED

- **O3 — The predicate is pure and carries at least two meaningful assertions across its tests.**
  - *Claim:* `can_transition_to` performs no I/O and does not mutate `self`; its tests assert both allowed and rejected pairs.
  - *Evidence collected:* signature is `pub fn can_transition_to(&self, target: &UserStatus) -> bool` — no `await`, no repo/port call, no `&mut self`, a pure `match`. The test module carries nine matrix assertions plus four terminal assertions — well over two, covering both allowed and rejected pairs.
  - *Status:* ☑ SATISFIED

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean.
  - *Evidence collected:* orchestrator ran `cargo fmt --check` → exit 0; `cargo clippy --workspace --all-targets -- -D warnings` → clean; `cargo nextest run --workspace` → 287 passed, 27 skipped, 0 failed. No new numeric bound introduced.
  - *Status:* ☑ SATISFIED

- **O5 — Reviewable: every diagram edge maps to true, every off-diagram pair to false.**
  - *Claim:* a reviewer reading the truth-table test confirms each drawn edge is `true` and `Deleted` has no outgoing edge.
  - *Evidence collected:* the 9-cell matrix maps `Active/Suspended` self-loops and `Active↔Suspended` and `*→Deleted` (non-Deleted `*`) to `true`, and all three `Deleted →` rows to `false`, matching the 01-domain-model lifecycle (Deleted terminal). Cross-checked against the match arms; the mapping is complete.
  - *Status:* ☑ SATISFIED

## Regression check

- No existing caller yet — `can_transition_to` is newly added and consumed only by Task 03. Nothing in scope to regress; the additive change left `UserStatus` variants and serde representation unchanged and the full suite green : ☑ PRESERVED

## Residue

- None noted.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: All five obligations SATISFIED — `UserStatus::can_transition_to` is a pure, total, wildcard-free predicate encoding the lifecycle (Deleted strictly terminal with no self-loop; Active↔Suspended and self-loops otherwise), verified by a full 9-cell truth-table test plus a negative-space terminal test, with fmt/clippy(--all-targets)/nextest clean (287/287). Discharged by the orchestrator (session model) after the build workflow's implementer connection dropped mid-report (the code and tests were written and intact); gate 1 (correctness) was performed inline by the orchestrator, which did not write the code.
