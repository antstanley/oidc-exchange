# Done Certificate — Task 03: canonical guidelines edits + merge-plan housekeeping

**Task:** [03-canonical_edits_and_merge.md](03-canonical_edits_and_merge.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-06-29

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
  - *Status:* ☑ SATISFIED — `.specs/development-guidelines.md` §Toolchain has the row
    `| mypy | latest, strict | uv run mypy over bindings/python; runs in CI |`, matching the
    landed gate: `bindings/python/pyproject.toml` `[tool.mypy] strict = true` (task 02) and the
    `.github/workflows/ci.yml` "Type-check" step `uv run mypy python` (working-directory
    `bindings/python`). §Repository hygiene carries the pre-push-hook bullet, matching the
    committed executable `.githooks/pre-push` (task 01). Check: the page asserts NO hard
    `clippy::too_many_lines` lint — the 70-line limit reads as a review gate that was "evaluated
    and declined"; `grep -rn too_many_lines` over `*.toml`/`*.rs`/`*.yml` (excluding `.specs`)
    returns none. No gate claimed that was not built.

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
  - *Status:* ☑ SATISFIED — §Open questions diff: the three resolved entries (pre-push/pre-commit
    hook, Python static type checker, function-size/two-assertions lint) were removed and only the
    unrelated `clippy` pedantic-ruleset entry remains (4 → 1, exactly the three resolved removed —
    negative-space holds). §Repository hygiene records the 70-line and assertion-density limits as
    review gates with the declined-`too_many_lines`@70 rationale (existing functions exceed 70
    lines, up to 134); a §Decisions "Local enforcement gates" entry records the resolution
    (committed pre-push hook via `core.hooksPath`, strict mypy in CI, limits kept as review gates).

- **O3 — The change spec is merged and the README updated.**
  - *Claim:* the change spec header reads `Status: Merged` with `Merged: 2026-06-29`, the file lives
    at `.specs/changes/merged/2026-06-24-add_local_enforcement_gates.md` (not the old path), and
    `.specs/README.md`'s Change-specs table row points at the merged path with `Merged` status.
  - *Evidence to collect:* confirm `.specs/changes/merged/2026-06-24-add_local_enforcement_gates.md`
    exists and the old `.specs/changes/2026-06-24-add_local_enforcement_gates.md` does not; read its
    header for `Status: Merged` + `Merged: 2026-06-29`; read the `.specs/README.md` Change-specs
    table row and confirm it links the merged path and shows `Merged`.
  - *Status:* ☑ SATISFIED — the file exists at
    `.specs/changes/merged/2026-06-24-add_local_enforcement_gates.md` and the old
    `.specs/changes/2026-06-24-add_local_enforcement_gates.md` is gone (`test -f` → absent; jj diff
    shows a rename). Header reads `Status: Merged · Date: 2026-06-24 · Merged: 2026-06-29`. The
    `.specs/README.md` Change-specs table row links `changes/merged/...` with `Merged` status.

- **O4 — Meets the repo definition of done for a docs-only change.**
  - *Claim:* all internal links still resolve after the move, and the page Date is bumped.
  - *Evidence to collect:* the moved change spec's `../development-guidelines.md` links now resolve
    from `changes/merged/` (so they must be `../../development-guidelines.md`) — read the moved file
    and check each relative link target exists; confirm `.specs/README.md`'s row link resolves;
    confirm `development-guidelines.md` header `Date:` reads `2026-06-29`.
  - *Checks:* resolve every changed relative link to a real file on disk (the move changed the
    change spec's depth by one segment).
  - *Status:* ☑ SATISFIED — the moved spec's two internal `development-guidelines.md` links were
    rewritten `../development-guidelines.md` → `../../development-guidelines.md` (lines 13 & 29) and
    resolve to `.specs/development-guidelines.md` (`test -f` OK from `changes/merged/`); the
    `.specs/README.md` Change-specs row link resolves (`test -f` OK from `.specs/`); the
    `development-guidelines.md` header `Date:` is bumped to 2026-06-29. NOTE: O4/DoD scope only
    these two link sets — both resolve. The move's broader collateral link breakage (other pages
    that link the change spec by its old path) is out of this obligation's scope and is captured in
    the Regression check below.

- **O5 — Reviewable: a reviewer confirms the reconciliation end to end (Reviewable).**
  - *Claim:* a reviewer can read the updated `development-guidelines.md`, confirm the gates are
    facts and the resolved questions gone; confirm the change spec is in `.specs/changes/merged/`
    with `Merged` status; and confirm the README table row is updated.
  - *Evidence to collect:* open `.specs/development-guidelines.md`, `.specs/changes/merged/2026-06-24-add_local_enforcement_gates.md`,
    and `.specs/README.md`; walk the three and confirm each matches its obligation above.
  - *Status:* ☑ SATISFIED — walked all three end to end: `.specs/development-guidelines.md` (gates
    recorded as facts in §Toolchain/§Repository hygiene/§Decisions, the three resolved Open
    questions gone), `.specs/changes/merged/2026-06-24-add_local_enforcement_gates.md` (`Merged`
    status, internal links resolve), and `.specs/README.md` (Change-specs row repointed to the
    merged path with `Merged`). Each matches its obligation. (The walk also surfaced the
    reconciliation gap recorded in the Regression check, outside these three files.)

## Regression check

- The task edits `development-guidelines.md`, moves the change spec, and edits `.specs/README.md`.
  Trace one downstream reference: other pages/plans that link the change spec or the guidelines page
  still resolve (the plan's own `Source spec` link and the README index entries) : ☑ PRESERVED (regression found, then resolved by orchestrator fix-forward) —
  moving the change spec into `changes/merged/` left five references to the OLD path
  `changes/2026-06-24-add_local_enforcement_gates.md` dangling: `.specs/README.md`:50 (the **Plans**
  table's source-spec column), `.specs/plans/2026-06-29-add_local_enforcement_gates/plan.md`:3 & :18
  (the `Source spec` link / §Spec), and backlog tasks `01-pre_push_hook.md`:5,
  `02-python_type_checker.md`:5, `03-canonical_edits_and_merge.md`:5 (their `Implements:` links).
  All five resolved before the move and now 404. The README **Plans**-table dangling link violates
  P3 ("must not break existing relative links … in the README"). This is collateral OUTSIDE task
  03's DoD: the DoD scoped link-resolution to the moved-spec links + the README **Change-specs**
  row only (both resolve), the Plans table was left untouched by design, and no task ever scheduled
  updating the plan/backlog back-references — so it is a plan-scoping gap, not an implementation
  error by the task-03 builder. Fix-forward: repoint the README Plans-table source-spec link to
  `changes/merged/...`, then decide whether the plan-folder back-references should be repointed or
  accepted as historical (path-at-authoring) references.
- **RESOLVED 2026-06-29 (orchestrator fix-forward).** All five dangling link targets were repointed
  to `changes/merged/2026-06-24-add_local_enforcement_gates.md`: the README **Plans**-table source
  link, `plan.md`:3 & :18 (`Source spec`/§Spec), and backlog `01/02/03` `Implements:`. Re-verified
  with `grep` — no markdown link target resolves to the old change-spec path; the merged target
  exists. Regression cleared.

## Residue

- The page's §Toolchain "CI" row already lists the four CI jobs; whether to also mention the new
  `python-typecheck` step there is a nicety, not an obligation.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE (NOT_DONE at first discharge; the sole REGRESSION was resolved by the orchestrator fix-forward recorded in the Regression check — all five dangling links repointed to the merged path and re-verified)
CONFIDENCE: high
SUMMARY: All five obligations O1–O5 are SATISFIED with evidence — the guidelines page records the
shipped gates accurately (mypy-strict row, pre-push-hook bullet, NO hard `too_many_lines` lint),
exactly the three resolved Open questions were removed with the pedantic-ruleset one kept and a
Decisions entry added, the change spec is flipped to `Merged` and moved to `changes/merged/` with
the README Change-specs row and the moved-spec/README links all resolving and the Date bumped — but
the regression check found a REGRESSION (the move left the README Plans-table source-spec link plus
the plan.md and backlog 01/02/03 `Implements:` back-references dangling on the old change-spec path),
which by the rubric makes this NOT_DONE; the breakage is out-of-DoD-scope collateral (a plan-scoping
gap, not a builder error) and is trivially fixed by repointing those references to the merged path.
