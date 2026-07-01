# Task 03 — canonical edits + merge

**Plan:** [plan.md](../plan.md) · **Certificate:** [03-canonical_edits_and_merge-certificate.md](03-canonical_edits_and_merge-certificate.md)

**Implements:** [.specs/changes/merged/2026-06-29-add_npm_trusted_publishing.md](../../../changes/merged/2026-06-29-add_npm_trusted_publishing.md) §Proposed changes (all four blocks) and §Merge plan; edits [.specs/bindings/specs/05-distribution.md](../../../bindings/specs/05-distribution.md) (Release pipeline, Artifacts, Assumptions / Decisions) and [.specs/bindings/specs/02-nodejs.md](../../../bindings/specs/02-nodejs.md) (Distribution).
**Depends on:** 01, 02
**Produces:** the canonical pages describe the shipped pipeline — 05-distribution.md's Release-pipeline prose names the `build-nodejs` → `publish-npm` pair and trusted publishing, its Artifacts row lists `@oidc-exchange/node` + the four platform packages with "OIDC trusted publishing, provenance", and its Assumptions/Decisions drop npm from the repository-secrets assumption and add an npm-trusted-publishing Decision; 02-nodejs.md's Distribution notes the `optionalDependencies` + `napi artifacts` mechanism; the change spec is flipped to `Merged`, stamped, and moved to `.specs/changes/merged/`; `.specs/README.md`'s Changes and Plans tables are updated.
**Pointers:** `.specs/bindings/specs/05-distribution.md:14` (Artifacts npm row), `:41` (Release-pipeline `build-nodejs`/`publish-nodejs` sentence), `:56` (secrets assumption), `:60` (Decisions); `.specs/bindings/specs/02-nodejs.md:42` (Distribution); `.specs/changes/merged/2026-06-29-add_npm_trusted_publishing.md` (Status/Merged stamp + move); `.specs/README.md:40` (Changes row), `:49` (Plans table).

## Steps

- [ ] Apply the §Release pipeline block to `05-distribution.md`: replace the `build-nodejs` + `publish-nodejs` sentence with the `build-nodejs` → `publish-npm` description (separate jobs, `id-token: write`, `publish` Environment, `napi artifacts`, `publint`/`@arethetypeswrong/cli`, `--provenance`, SHA pins, Node ≥ 24.8.0); bump the page `**Date:**`.
- [ ] Update the `05-distribution.md` Artifacts table npm row to `@oidc-exchange/node` + the four named platform packages, channel "npm (OIDC trusted publishing, provenance)".
- [ ] In `05-distribution.md` Assumptions/Decisions, drop npm from the "credentials configured as repository secrets" assumption and add the *npm trusted publishing* Decision from the change spec.
- [ ] Apply the §Distribution block to `02-nodejs.md`: append the note that `package.json` declares the four platform packages as `optionalDependencies` populated by `napi artifacts`, and that npm installs only the host-matching entry which the loader resolves (local `oidc-exchange.node` fallback); bump its `**Date:**`.
- [ ] Discharge the change spec's Merge plan: flip `**Status:**` to `Merged`, add a `**Merged:** 2026-06-30` stamp, and move the file to `.specs/changes/merged/2026-06-29-add_npm_trusted_publishing.md` (use `jj` to move so history follows).
- [ ] Update `.specs/README.md`: change the npm change-spec row to point at `changes/merged/...` with Status `Merged`, and add this plan to the Plans table.

## Definition of done

- [ ] `05-distribution.md` Release-pipeline prose, Artifacts npm row, and Assumptions/Decisions all match the change spec's Proposed-changes blocks (separate build/publish jobs, trusted publishing, provenance, named platform-package set, secrets assumption no longer claims npm) — and describe the pipeline task 02 actually shipped, not a divergent one.
- [ ] `02-nodejs.md` Distribution records the `optionalDependencies` + `napi artifacts` mechanism, consistent with what task 01 added to `package.json`.
- [ ] Both edited pages have a bumped `**Date:**`; no stale "publish-nodejs" / "NPM_TOKEN as a secret" claim survives on either page (negative space: grep the two pages for `publish-nodejs` and the secrets-assumption wording returns nothing).
- [ ] The change spec is `Merged`-stamped and lives at `.specs/changes/merged/2026-06-29-add_npm_trusted_publishing.md`; `.specs/README.md` references it under `merged/` and lists this plan with its status.
- [ ] Meets the repo definition of done for what the task touches: docs/spec only, so no test suite runs; the edits are internally consistent and every cross-link still resolves (the moved change spec's inbound links are updated).
- [ ] Reviewable: a reviewer reads the two canonical pages against the shipped `package.json` (task 01) and `release.yml` (task 02) and confirms the spec now describes reality with no surviving secret/`publish-nodejs` reference, and that the change spec + README reflect the merge.

## Open questions

- None at the task level; the external trusted-publisher registration and staged-publish approval are recorded in `plan.md`'s Open questions as out-of-repo follow-ups, not blockers for these edits.
