# Task 03 — canonical edits + merge

**Plan:** [plan.md](../plan.md) · **Certificate:** [03-canonical_edits_and_merge-certificate.md](03-canonical_edits_and_merge-certificate.md)

**Implements:** [.specs/changes/2026-06-29-add_pypi_trusted_publishing.md](../../../changes/2026-06-29-add_pypi_trusted_publishing.md) §Proposed changes (all three blocks) and §Merge plan; edits [.specs/bindings/specs/05-distribution.md](../../../bindings/specs/05-distribution.md) (Release pipeline, Assumptions / Decisions) and [.specs/bindings/specs/03-python.md](../../../bindings/specs/03-python.md) (Distribution).
**Depends on:** 01, 02
**Produces:** the canonical pages describe the shipped pipeline — 05-distribution.md's Release-pipeline prose names the `build-python` (manylinux abi3 wheels + sdist) → `publish-pypi` (trusted publishing) pair, and its Assumptions/Decisions drop PyPI from the repository-secrets assumption and add a PyPI-trusted-publishing Decision; 03-python.md's Distribution records the `pyo3/abi3-py310` maturin feature and the manylinux_2_28 build with an sdist; the change spec is flipped to `Merged`, stamped, and moved to `.specs/changes/merged/`; `.specs/README.md`'s Changes and Plans tables are updated.
**Pointers:** `.specs/bindings/specs/05-distribution.md:15` (Artifacts PyPI wheels row), `:42` (Release-pipeline `build-python`/`publish-python` sentence), `:56` (secrets assumption), `:62` (Decisions); `.specs/bindings/specs/03-python.md:44`-`:48` (Distribution paragraph); `.specs/changes/2026-06-29-add_pypi_trusted_publishing.md` (Status/Merged stamp + move); `.specs/README.md:41` (Changes row), `:49`-`:53` (Plans table).

## Steps

- [ ] Apply the §Release pipeline block to `05-distribution.md`: replace the `build-python` + `publish-python` sentence with the `build-python` → `publish-pypi` description (manylinux_2_28 abi3 wheels via `PyO3/maturin-action`, sdist, separate jobs, `id-token: write`, `pypi` Environment, `pypa/gh-action-pypi-publish`, SHA pins, no `PYPI_TOKEN`); bump the page `**Date:**`. Coordinate with the npm plan's edit to the same Release-pipeline paragraph — keep both the npm and PyPI changes, do not overwrite.
- [ ] In `05-distribution.md` Assumptions/Decisions, drop PyPI from the "credentials configured as repository secrets" assumption and add the *PyPI trusted publishing* Decision from the change spec, leaving the npm-trusted-publishing edits (if the npm plan landed) intact.
- [ ] Apply the §Distribution block to `03-python.md`: replace the Distribution paragraph with the change spec's wording — `pyproject.toml` enables `pyo3/abi3-py310`, one wheel per platform spans 3.10–3.13, Linux wheels built in a `manylinux_2_28` container, platforms `manylinux_2_28_{x86_64,aarch64}` / `win_amd64` / `macosx_11_0_arm64`, an sdist published alongside; bump its `**Date:**`.
- [ ] Discharge the change spec's Merge plan: flip `**Status:**` to `Merged`, add a `**Merged:** 2026-06-30` stamp, and move the file to `.specs/changes/merged/2026-06-29-add_pypi_trusted_publishing.md` (use `jj` to move so history follows).
- [ ] Update `.specs/README.md`: change the PyPI change-spec row to point at `changes/merged/...` with Status `Merged`, and add this plan to the Plans table.

## Definition of done

- [ ] `05-distribution.md` Release-pipeline prose and Assumptions/Decisions match the change spec's Proposed-changes blocks (manylinux abi3 wheels + sdist, separate build/publish jobs, trusted publishing, secrets assumption no longer claims PyPI) — and describe the pipeline task 02 actually shipped, not a divergent one — without reverting the npm plan's edits to the same page.
- [ ] `03-python.md` Distribution records the `pyo3/abi3-py310` feature, the manylinux_2_28 build, the four target platforms, and the sdist — consistent with what task 01 added to `pyproject.toml` and what task 02 shipped in the workflow.
- [ ] Both edited pages have a bumped `**Date:**`; no stale "publish-python" / "PyPI credentials as a secret" / "PYPI_TOKEN" claim survives on either page (negative space: grep the two pages for `publish-python`, `PYPI_TOKEN`, and the secrets-assumption wording returns nothing).
- [ ] The change spec is `Merged`-stamped and lives at `.specs/changes/merged/2026-06-29-add_pypi_trusted_publishing.md`; `.specs/README.md` references it under `merged/` and lists this plan with its status.
- [ ] Meets the repo definition of done for what the task touches: docs/spec only, so no test suite runs; the edits are internally consistent and every cross-link still resolves (the moved change spec's inbound links are updated).
- [ ] Reviewable: a reviewer reads the two canonical pages against the shipped `pyproject.toml` (task 01) and `release.yml` (task 02) and confirms the spec now describes reality with no surviving secret/`publish-python` reference, and that the change spec + README reflect the merge.

## Open questions

- None at the task level; the external trusted-publisher registration is recorded in `plan.md`'s Open questions as an out-of-repo follow-up, and the shared-`release.yml`/`05-distribution.md` coordination with the npm plan is recorded there too — neither blocks these edits.
