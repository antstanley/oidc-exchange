# Done Certificate — Task 02: canonical Open-question removal and merge-plan housekeeping

**Task:** [02-canonical_and_housekeeping.md](02-canonical_and_housekeeping.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-06-29 — unverified   <!-- validator sets: Validated YYYY-MM-DD -->

> This certificate is a verification protocol for Task 02. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 02) ≡ every obligation O1…O5 below holds, each backed by the evidence the obligation
names (a file read, a grep result, a path check) — not by assertion.

## Premises

- **P1 — Goal.** The task produces a canonical 06-configuration with the resolved Open question
  removed and Date bumped, and a change spec marked Merged, moved to `changes/merged/`, and
  re-pointed in `.specs/README.md`.
- **P2 — Obligations.** The task is done iff O1…O5 all hold. One Oi per definition-of-done item, in
  DoD order; O5 is the `Reviewable:` item.
- **P3 — Invariants.** Must not alter any of 06-configuration's body beyond the Open-question removal
  and the Date bump; must not orphan any link to the change spec; must not change other rows of the
  `.specs/README.md` Change specs table. Depends on Task 01 having actually applied the sweep — the
  "resolved by this change" / `Merged` claims must be true, not aspirational.

## Obligations

- **O1 — 06-configuration drops the stale Open question and bumps its Date.**
  - *Claim:* `.specs/service/specs/06-configuration.md` no longer contains the stale-`cloudtrail`
    Open question, its `**Date:**` reads `2026-06-29`, and the rest of the page is otherwise unchanged.
  - *Evidence to collect:* run `rg -n 'cloudtrail|file.*webhook|swept' .specs/service/specs/06-configuration.md`
    — expect no Open-question hit; read the header line and confirm `**Date:** 2026-06-29` with
    `**Status:** Implemented` unchanged; if the Open question was the only bullet, confirm the
    `### Open questions` heading remains with a single `- None.` (no dangling empty heading); diff the
    page against its pre-task version and confirm the only changes are the Date and the Open-question
    block.
  - *Status:* ☐ unverified

- **O2 — The change spec is moved and re-stamped as Merged.**
  - *Claim:* the change spec now lives at `.specs/changes/merged/2026-06-24-cleanup_stale_references.md`
    with `**Status:** Merged` and `**Merged:** 2026-06-29`, and no copy remains at the old path.
  - *Evidence to collect:* confirm `.specs/changes/merged/2026-06-24-cleanup_stale_references.md`
    exists and read its header line for `**Status:** Merged` and `**Merged:** 2026-06-29`; confirm
    `.specs/changes/2026-06-24-cleanup_stale_references.md` (the un-merged path) no longer exists
    (`test ! -e`); run `jj st` and confirm the rename is tracked (no stray duplicate).
  - *Status:* ☐ unverified

- **O3 — `.specs/README.md` reflects the merge, with no link left at the old path.**
  - *Claim:* the Change specs table row reads `Merged` and links to `changes/merged/...`; no `.specs`
    link still points at the old un-merged path.
  - *Evidence to collect:* read the `2026-06-24-cleanup_stale_references` row of the
    `.specs/README.md` Change specs table — expect Status `Merged` and a
    `changes/merged/2026-06-24-cleanup_stale_references.md` link; run
    `rg -n '2026-06-24-cleanup_stale_references' .specs/` and confirm every hit either targets the
    `changes/merged/` path or is inside the moved file / the plan folder — none points at the old
    `changes/2026-06-24-cleanup_stale_references.md` path.
  - *Checks:* confirm the plan's own `Source spec` link (and any task `Implements` link) to the change
    spec still resolves after the move — they should now include the `merged/` segment or be
    acknowledged in O3's grep as intentionally pointing at the new path.
  - *Status:* ☐ unverified

- **O4 — Meets the repo definition of done for a docs change.**
  - *Claim:* the edited Markdown is well-formed, all touched links resolve, and the change is
    described with its why.
  - *Evidence to collect:* per `.specs/development-guidelines.md` §Definition of done, confirm the
    Markdown renders (the Astro site still builds — `pnpm --filter ./apps/website build` if the
    moved/edited pages are in the docs tree; the `.specs/` pages are not part of the site, so a link
    check suffices for them); resolve each link touched in O1–O3 by path and confirm the target file
    exists. (No code, no tests, no Rust — those DoD rows do not apply.)
  - *Status:* ☐ unverified

- **O5 — Reviewable: Merged header, clean canonical page, and a resolving README link (Reviewable).**
  - *Claim:* a reviewer can open the moved change spec and see the Merged header, grep
    06-configuration and find no Open-question cloudtrail hit, and follow the `.specs/README.md`
    table link to the `changes/merged/` file.
  - *Evidence to collect:* open `.specs/changes/merged/2026-06-24-cleanup_stale_references.md`
    (expect Merged header); run `rg -n 'cloudtrail' .specs/service/specs/06-configuration.md` (expect
    no Open-question hit); follow the `.specs/README.md` Change specs row link and confirm it resolves
    to the `changes/merged/` path.
  - *Status:* ☐ unverified

## Regression check

This task edits only `.specs/` documents (one canonical page, one change spec, one index); it touches
no runtime code or call paths.

- Other rows of the `.specs/README.md` Change specs table are unchanged (only the
  `cleanup_stale_references` row's Status/link move) → expect the other five rows byte-for-byte
  identical : ☐ (PRESERVED / REGRESSION)
- 06-configuration's body outside the Date line and the Open-questions block is unchanged → expect
  the `[audit]` / `[providers.<name>]` enumerations and all other sections intact : ☐ (PRESERVED / REGRESSION)

## Residue

Notes for the validator, not obligations:

- This task asserts the sweep (Task 01) is done; if O1 of Task 01's certificate is not SATISFIED, the
  `Merged` / "resolved by this change" claims here are premature — flag rather than pass.
- No schema change is involved (the change spec's Merge plan item 2 is "No schema change"); do not
  expect a `canonical-types.schema.json` edit.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐ <one sentence deriving the verdict from the statuses>
