# Task 01 — Index 2026-08-05 change specs

**Plan:** [plan.md](../plan.md) · **Certificate:** Not authored by request.

**Implements:** [PR #28 index-only intent](../plan.md#source-and-definition-of-done-baseline): index the fourteen 2026-08-05 change-spec proposals in `.specs/README.md` `## Change specs`.
**Depends on:** —
**Produces:** the canonical index lists each intended 2026-08-05 change-spec proposal once, with a relative link that resolves from `.specs/README.md` when the source spec set is present.
**Pointers:** `.specs/README.md:28-50`; branch diff `trunk()..spec/index-2026-08-05-change-specs`.

## Steps

- [ ] Add the fourteen `changes/2026-08-05-*.md` rows from the branch diff to the Proposed portion of `.specs/README.md` `## Change specs`, after the existing 2026-06-24 proposed rows and before merged entries.
- [ ] Preserve the exact row labels, status `Proposed`, target summaries, and `changes/...` relative-link form from the branch diff.
- [ ] Do not add, edit, or implement any change spec, source code, configuration, workflow, or admin JWT behavior.
- [ ] Inspect the rendered Markdown/table structure and validate each planned target path against the intended source spec set, including intentional unresolved links and the absence of certificates.
- [ ] Confirm the plan DAG matches the single-task review scope and remains index-only.

## Definition of done

- [ ] `.specs/README.md` contains exactly the fourteen 2026-08-05 entries in the branch diff, each once and each marked `Proposed`.
- [ ] Each new `changes/2026-08-05-*.md` relative link has valid Markdown syntax and targets the expected source path when the intended spec set is present; unresolved links are documented as expected in this checkout.
- [ ] The diff is confined to the index entry update; it contains no admin JWT implementation work or other change-spec/source changes.
- [ ] Meets the documentation-only portion of the repo definition of done: repository hygiene is preserved and changed Markdown links are checked (see plan.md baseline).
- [ ] Reviewable: compare `.specs/README.md` with the branch diff and follow the fourteen indexed links in the intended spec set.
