# Task 03 — Canonical guidelines edits + merge-plan housekeeping

**Plan:** [plan.md](../plan.md) · **Certificate:** [03-canonical_edits_and_merge-certificate.md](03-canonical_edits_and_merge-certificate.md)

**Implements:** [.specs/changes/2026-06-24-add_local_enforcement_gates.md](../../../changes/2026-06-24-add_local_enforcement_gates.md) §Proposed changes (all three blocks), §Implementation notes 3 & 4 (function-size + assertion-density resolved as documented review gates), and §Merge plan; updates [.specs/development-guidelines.md](../../../development-guidelines.md) §Toolchain / §Repository hygiene / §Open questions and [.specs/README.md](../../../README.md).
**Depends on:** 01, 02
**Produces:** the updated `development-guidelines.md` page recording the landed gates as facts and resolving the limit-lint open questions; the change spec flipped to `Merged` and moved to `.specs/changes/merged/`; and the `.specs/README.md` Change-specs table updated.
**Pointers:** `.specs/development-guidelines.md` (§Toolchain table ~L14–31, §Repository hygiene ~L283–294, §Decisions ~L344–356, §Open questions ~L358–367); `.specs/changes/2026-06-24-add_local_enforcement_gates.md` header + new `.specs/changes/merged/`; `.specs/README.md` Change-specs table ~L33–40.

## Steps

- [ ] In `development-guidelines.md` §Toolchain, add a row: `| mypy | latest, strict | uv run mypy over bindings/python; runs in CI |` (matching the gate that landed in task 02).
- [ ] In §Repository hygiene, add the pre-push-hook bullet from the change spec (the hook runs format-check, lint, and the fast test tier for every language the change touches; CI re-runs the same plus the slow tier; a failing hook blocks the push; do not bypass it), reflecting the committed hook from task 01.
- [ ] In §Repository hygiene (or the relevant Rust/limits note), record that the **70-lines-per-function** limit stays a **review gate**: note that enabling a hard `clippy::too_many_lines` lint at threshold 70 was evaluated and declined because existing functions exceed it, and that **assertion density** likewise stays a review-only gate (not lintable off the shelf).
- [ ] In §Open questions, remove the three now-resolved entries — the pre-push/pre-commit hook question, the Python-type-checker question, and the function-size/two-assertions lint question — and leave the unrelated `clippy` pedantic-ruleset question in place.
- [ ] Add a §Decisions entry recording the wired gates (pre-push hook via `core.hooksPath`, mypy strict in CI, limit lints kept as documented review gates) so the resolution is captured, not just deleted from Open questions.
- [ ] Bump the `development-guidelines.md` header `**Date:**` to 2026-06-29.
- [ ] Flip the change spec header `**Status:** Proposed` to `**Status:** Merged` and add `**Merged:** 2026-06-29`; create `.specs/changes/merged/` and move the change-spec file there.
- [ ] Update `.specs/README.md`'s Change-specs table row for this change to point at `changes/merged/2026-06-24-add_local_enforcement_gates.md` and show `Merged` status.

## Definition of done

- [ ] `development-guidelines.md` shows mypy in the Toolchain table and the pre-push-hook bullet in Repository hygiene, both matching the gates that actually shipped in tasks 01–02 (no claim of a gate that was not built — e.g. no hard `too_many_lines` lint is asserted).
- [ ] The three resolved Open questions are gone, the limit-lint and assertion-density review-gate rationale is documented, the unrelated pedantic-ruleset question remains, and a Decisions entry records the resolution (negative-space: nothing genuinely undecided was deleted).
- [ ] The change spec reads `Status: Merged` with `Merged: 2026-06-29` and lives at `.specs/changes/merged/2026-06-24-add_local_enforcement_gates.md` (not at the old path); `.specs/README.md` references the merged path with `Merged` status.
- [ ] Meets the repo definition of done for a docs-only change: all internal links still resolve from their new locations (the moved change spec's `../development-guidelines.md` relative links, the README table link), and the page `Date` is bumped.
- [ ] Reviewable: a reviewer reads the updated `development-guidelines.md`, confirms the gates are recorded as facts and the resolved questions are gone; confirms the change spec sits in `.specs/changes/merged/` with `Merged` status; and confirms the `.specs/README.md` table row is updated.

## Open questions

- None beyond the plan-level ones.
