# Task 02 — Remove resolved Open question and run the merge-plan housekeeping

**Plan:** [plan.md](../plan.md) · **Certificate:** [02-canonical_and_housekeeping-certificate.md](02-canonical_and_housekeeping-certificate.md)

**Implements:** [changes/merged/2026-06-24-cleanup_stale_references.md](../../../changes/merged/2026-06-24-cleanup_stale_references.md) §Proposed changes (06-configuration Open question removal) and §Merge plan; edits [service/specs/06-configuration.md](../../../service/specs/06-configuration.md) §Open questions
**Depends on:** 01
**Produces:** the resolved Open question is gone from 06-configuration with a bumped Date, and the change spec is marked Merged, moved to `changes/merged/`, and re-pointed in `.specs/README.md`
**Pointers:** `.specs/service/specs/06-configuration.md:3` (Date), `.specs/service/specs/06-configuration.md:122` (Open questions), `.specs/changes/2026-06-24-cleanup_stale_references.md:3` (Status), `.specs/README.md:33` (Change specs table)

## Steps

- [ ] `.specs/service/specs/06-configuration.md` — remove the Open-questions bullet about stale `cloudtrail`/`file`/`webhook` audit adapters and the sweep; if it is the only bullet, leave the `### Open questions` heading with a single `- None.` so the closing block stays well-formed.
- [ ] `.specs/service/specs/06-configuration.md` — bump the header `**Date:**` from `2026-06-24` to `2026-06-29` (leave `**Status:** Implemented` unchanged).
- [ ] `.specs/changes/2026-06-24-cleanup_stale_references.md` — flip `**Status:** Proposed` to `**Status:** Merged` and add `**Merged:** 2026-06-29` to the header line.
- [ ] Move the change spec into the merged folder with the VCS (so history is preserved): `jj` tracks the rename automatically — `mkdir -p .specs/changes/merged && git mv .specs/changes/2026-06-24-cleanup_stale_references.md .specs/changes/merged/` (or a plain `mv`; jj snapshots either way).
- [ ] `.specs/README.md` — in the Change specs table update this row: change Status `Proposed` → `Merged` and re-point the link from `changes/2026-06-24-cleanup_stale_references.md` to `changes/merged/2026-06-24-cleanup_stale_references.md`.
- [ ] Verify no other `.specs` link points at the old (pre-move) change-spec path: `rg -n '2026-06-24-cleanup_stale_references' .specs/` and fix any stragglers (the plan's own Source-spec link may now need the `merged/` segment).

## Definition of done

- [ ] `.specs/service/specs/06-configuration.md` no longer contains the stale-`cloudtrail` Open question and its `**Date:**` reads `2026-06-29`; the rest of the page is byte-for-byte unchanged.
- [ ] The change spec now lives at `.specs/changes/merged/2026-06-24-cleanup_stale_references.md`, with `**Status:** Merged` and `**Merged:** 2026-06-29`, and no copy remains at the old path.
- [ ] `.specs/README.md`'s Change specs table shows the row as `Merged` and links to the `changes/merged/...` path; `rg -n '2026-06-24-cleanup_stale_references' .specs/` shows no link still pointing at the old un-merged path.
- [ ] Meets the repo definition of done for a docs change (see plan.md baseline): Markdown well-formed, links resolve, change description states the why.
- [ ] Reviewable: open the moved change spec (Merged header), grep `06-configuration.md` for `cloudtrail` (no Open-question hit), and follow the `.specs/README.md` table link to confirm it resolves to `changes/merged/`.

## Open questions

- None.
