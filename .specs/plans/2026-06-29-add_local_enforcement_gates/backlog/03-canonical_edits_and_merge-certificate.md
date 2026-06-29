# Done Certificate — Task 03: canonical guidelines edits + merge-plan housekeeping

**Task:** [03-canonical_edits_and_merge.md](03-canonical_edits_and_merge.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-06-29 — unverified   <!-- validator sets: Validated YYYY-MM-DD -->

> This certificate is a verification protocol for Task 03. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 03) ≡ every obligation O1…O5 below holds, each backed by the evidence the obligation
names (a file location or a resolved link) — not by assertion.

## Premises

- **P1 — Goal.** The task updates `development-guidelines.md` to record the landed gates as facts
  and resolve the limit-lint questions, then performs the change-spec merge housekeeping (Status,
  Merged date, move to `merged/`, README update).
- **P2 — Obligations.** Done iff O1…O5 all hold. One Oi per definition-of-done item, in DoD
  order; O5 is the `Reviewable:` item.
- **P3 — Invariants.** Must not assert any gate that did not actually ship (tasks 01–02 are the
  source of truth for what landed); must not delete a genuinely-undecided Open question; must not
  break existing relative links in the moved change spec or the README.

## Obligations

- **O1 — The guidelines page records the gates that shipped, accurately.**
  - *Claim:* `development-guidelines.md` §Toolchain has a `mypy … strict … bindings/python; runs in
    CI` row, and §Repository hygiene has the pre-push-hook bullet — both matching the gates built in
    tasks 01–02, with no claim of a gate that was not built.
  - *Evidence to collect:* read `.specs/development-guidelines.md` §Toolchain and §Repository
    hygiene; confirm the mypy row and the pre-push bullet are present. Cross-check against the
    actual artifacts: `.githooks/pre-push` (task 01) and the `[tool.mypy]` + CI step (task 02).
  - *Checks:* confirm the page does **not** assert a hard `clippy::too_many_lines` lint (which was
    declined) — the function-size limit must read as a review gate, not a wired lint.
  - *Status:* ☐ unverified

- **O2 — Resolved Open questions removed; limit-lint rationale documented; nothing undecided lost.**
  - *Claim:* the pre-push-hook, Python-type-checker, and function-size/two-assertions Open
    questions are removed; the 70-line and assertion-density limits are documented as review gates
    (with the reason the size lint was declined); the unrelated clippy-pedantic-ruleset Open
    question remains; a Decisions entry records the resolution.
  - *Evidence to collect:* read §Open questions — confirm the three resolved entries are gone and
    the pedantic-ruleset entry remains; read §Repository hygiene / §Decisions — confirm the
    review-gate rationale for function-size and assertion-density and a Decisions entry recording
    the wired gates.
  - *Checks:* diff the Open-questions list against the pre-change page (four entries → one
    remaining) — confirm exactly the three resolved ones were removed, no more (negative-space).
  - *Status:* ☐ unverified

- **O3 — The change spec is merged and the README updated.**
  - *Claim:* the change spec header reads `Status: Merged` with `Merged: 2026-06-29`, the file lives
    at `.specs/changes/merged/2026-06-24-add_local_enforcement_gates.md` (not the old path), and
    `.specs/README.md`'s Change-specs table row points at the merged path with `Merged` status.
  - *Evidence to collect:* confirm `.specs/changes/merged/2026-06-24-add_local_enforcement_gates.md`
    exists and the old `.specs/changes/2026-06-24-add_local_enforcement_gates.md` does not; read its
    header for `Status: Merged` + `Merged: 2026-06-29`; read the `.specs/README.md` Change-specs
    table row and confirm it links the merged path and shows `Merged`.
  - *Status:* ☐ unverified

- **O4 — Meets the repo definition of done for a docs-only change.**
  - *Claim:* all internal links still resolve after the move, and the page Date is bumped.
  - *Evidence to collect:* the moved change spec's `../development-guidelines.md` links now resolve
    from `changes/merged/` (so they must be `../../development-guidelines.md`) — read the moved file
    and check each relative link target exists; confirm `.specs/README.md`'s row link resolves;
    confirm `development-guidelines.md` header `Date:` reads `2026-06-29`.
  - *Checks:* resolve every changed relative link to a real file on disk (the move changed the
    change spec's depth by one segment).
  - *Status:* ☐ unverified

- **O5 — Reviewable: a reviewer confirms the reconciliation end to end (Reviewable).**
  - *Claim:* a reviewer can read the updated `development-guidelines.md`, confirm the gates are
    facts and the resolved questions gone; confirm the change spec is in `.specs/changes/merged/`
    with `Merged` status; and confirm the README table row is updated.
  - *Evidence to collect:* open `.specs/development-guidelines.md`, `.specs/changes/merged/2026-06-24-add_local_enforcement_gates.md`,
    and `.specs/README.md`; walk the three and confirm each matches its obligation above.
  - *Status:* ☐ unverified

## Regression check

- The task edits `development-guidelines.md`, moves the change spec, and edits `.specs/README.md`.
  Trace one downstream reference: other pages/plans that link the change spec or the guidelines page
  still resolve (the plan's own `Source spec` link and the README index entries) : ☐ (PRESERVED / REGRESSION)

## Residue

- The page's §Toolchain "CI" row already lists the four CI jobs; whether to also mention the new
  `python-typecheck` step there is a nicety, not an obligation.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐ <one sentence deriving the verdict from the statuses>
