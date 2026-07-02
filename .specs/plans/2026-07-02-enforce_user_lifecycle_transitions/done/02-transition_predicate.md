# Task 02 — transition predicate

**Plan:** [plan.md](../plan.md) · **Certificate:** [02-transition_predicate-certificate.md](02-transition_predicate-certificate.md)

**Implements:** [01-domain-model.md](../../../service/specs/01-domain-model.md) §Lifecycles → User status (state diagram and transition rules: `Deleted` terminal, `Suspended → Deleted` edge, same-status no-op except on `Deleted`)
**Depends on:** —
**Produces:** `UserStatus::can_transition_to`, a pure predicate encoding the lifecycle, with a full ordered-pair truth-table test
**Pointers:** `crates/core/src/domain/user.rs:27-35` (`UserStatus` enum)

## Steps

- [x] Add `UserStatus::can_transition_to(&self, target: &UserStatus) -> bool` to `crates/core/src/domain/user.rs`, next to the enum.
- [x] Encode the rules: `target == Deleted` is allowed from any non-`Deleted` current status; a same-status target is allowed (no-op) except when current is `Deleted`; `Active ↔ Suspended` are allowed; every other pair — and every transition out of `Deleted`, including `Deleted → Deleted` — is rejected.
- [x] Document the predicate's semantics in a doc comment that states which transitions are allowed, without paraphrasing each match arm.

## Definition of done

- [x] `can_transition_to` returns the correct verdict for all nine ordered `(current, target)` status pairs, asserted by a unit test enumerating the full matrix.
- [x] Negative-space is covered: `Deleted → Active`, `Deleted → Suspended`, and `Deleted → Deleted` all return `false` (no status patch leaves `Deleted`), and `Suspended → Deleted` returns `true`.
- [x] The predicate is pure (no I/O, no `self` mutation) and carries at least two meaningful assertions across its tests per the guidelines.
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: a reviewer reads the truth-table test and confirms every diagram edge maps to `true` and every off-diagram pair to `false`, with `Deleted` having no outgoing edge.
