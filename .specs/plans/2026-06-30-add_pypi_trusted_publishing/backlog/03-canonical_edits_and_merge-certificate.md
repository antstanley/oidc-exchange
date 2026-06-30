# Done Certificate — Task 03: canonical edits + merge

**Task:** [03-canonical_edits_and_merge.md](03-canonical_edits_and_merge.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-06-30 — unverified   <!-- validator sets: Validated YYYY-MM-DD -->

> This certificate is a verification protocol for Task 03. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 03) ≡ every obligation O1…O6 below holds, each backed by the evidence the obligation
names (a spec-page location, a grep result, or a file move) — not by assertion.

## Premises

- **P1 — Goal.** The task applies the change spec's three Proposed-changes blocks to the canonical
  pages (05-distribution.md Release pipeline / Assumptions-Decisions; 03-python.md Distribution),
  then discharges the Merge plan: the change spec flips to `Merged`, is stamped and moved to
  `.specs/changes/merged/`, and `.specs/README.md` is updated.
- **P2 — Obligations.** Done iff O1…O6 all hold. One Oi per definition-of-done item, in DoD order;
  O6 is the `Reviewable:` item.
- **P3 — Invariants.** The edits must describe the pipeline tasks 01 and 02 actually shipped (not a
  divergent one); the npm plan's edits to the same shared pages (05-distribution.md) must not be
  reverted; every inbound cross-link to the change spec must still resolve after the move; no
  unrelated spec page is changed.

## Obligations

- **O1 — 05-distribution.md matches the change spec's Proposed-changes blocks and reality.**
  - *Claim:* the Release-pipeline prose and Assumptions/Decisions in `05-distribution.md` reflect
    `build-python` (manylinux abi3 wheels + sdist) → `publish-pypi` (trusted publishing), and an
    Assumptions section that no longer claims PyPI uses a stored secret.
  - *Evidence to collect:* read `.specs/bindings/specs/05-distribution.md`; confirm the
    Release-pipeline paragraph names `build-python` → `publish-pypi` (manylinux_2_28 wheels via
    maturin-action, sdist, separate jobs, `id-token: write`, `pypi` Environment,
    `pypa/gh-action-pypi-publish`, SHA pins); confirm the Assumptions list no longer includes PyPI
    in "credentials configured as repository secrets" and a *PyPI trusted publishing* Decision is
    present.
  - *Checks:* cross-check the prose against the shipped `release.yml` (task 02) — the spec must
    describe the job that exists; confirm the npm plan's edits to this page (if landed) are intact.
  - *Status:* ☐ unverified

- **O2 — 03-python.md Distribution records abi3-py310 + manylinux + sdist.**
  - *Claim:* `03-python.md` §Distribution notes that `pyproject.toml` enables `pyo3/abi3-py310`,
    that one wheel per platform spans 3.10–3.13, that Linux wheels build in a `manylinux_2_28`
    container, names the four target platforms, and notes an sdist publishes alongside.
  - *Evidence to collect:* read `.specs/bindings/specs/03-python.md` §Distribution; confirm the
    replaced paragraph; cross-check the `abi3-py310` claim against the `[tool.maturin] features`
    task 01 added to `bindings/python/pyproject.toml`.
  - *Status:* ☐ unverified

- **O3 — Both pages bumped, no stale publish-python / PYPI_TOKEN claim survives.**
  - *Claim:* both edited pages carry a bumped `**Date:**`, and no `publish-python` /
    "PyPI credentials as a secret" / `PYPI_TOKEN` claim remains on either page.
  - *Evidence to collect:* `grep -nE 'publish-python|PYPI_TOKEN' .specs/bindings/specs/05-distribution.md
    .specs/bindings/specs/03-python.md` — expect no matches; read each page header and confirm
    `**Date:**` is `2026-06-30` (or later than its prior value).
  - *Status:* ☐ unverified

- **O4 — The change spec is Merged-stamped and relocated; README updated.**
  - *Claim:* the change spec is `Merged`-stamped and lives at
    `.specs/changes/merged/2026-06-29-add_pypi_trusted_publishing.md`; `.specs/README.md` references
    it under `merged/` and lists this plan.
  - *Evidence to collect:* confirm `.specs/changes/merged/2026-06-29-add_pypi_trusted_publishing.md`
    exists and `.specs/changes/2026-06-29-add_pypi_trusted_publishing.md` no longer does
    (`jj diff --name-only` shows the rename/move); read its header for `**Status:** Merged` and a
    `**Merged:**` stamp; read `.specs/README.md` and confirm the Changes row points at `merged/`
    with Status `Merged` and the Plans table lists `2026-06-30-add_pypi_trusted_publishing`.
  - *Status:* ☐ unverified

- **O5 — Meets the repo definition of done for what the task touches.**
  - *Claim:* docs/spec only — no test suite runs; the edits are internally consistent and every
    cross-link resolves.
  - *Evidence to collect:* per `.specs/development-guidelines.md` §Definition of done, confirm the
    change is docs-only via `jj diff --name-only` (only `.specs/**` Markdown changed); resolve each
    relative link the edits touch (the moved change spec's inbound links from `README.md` and the
    plan) and confirm none 404.
  - *Status:* ☐ unverified

- **O6 — Reviewable: a reviewer confirms the spec describes reality and the merge landed (Reviewable).**
  - *Claim:* a reviewer reads the two canonical pages against the shipped `pyproject.toml` (task 01)
    and `release.yml` (task 02) and confirms the spec now describes reality with no surviving
    secret/`publish-python` reference, and that the change spec + README reflect the merge.
  - *Evidence to collect:* read `05-distribution.md` and `03-python.md` side by side with
    `bindings/python/pyproject.toml` and `release.yml`; confirm each spec claim has a matching
    shipped fact; confirm the change spec sits under `merged/` and `README.md` links resolve.
  - *Status:* ☐ unverified

## Regression check

- `.specs/README.md` and any spec page linking the change spec must not break when it moves to
  `merged/`. Trace: every reference to `2026-06-29-add_pypi_trusted_publishing.md` resolves to the
  `merged/` path after the move : ☐ (PRESERVED / REGRESSION)
- The npm plan's edits to `05-distribution.md` must survive this task's edits to the same page.
  Trace: the npm Artifacts row, Release-pipeline npm prose, and npm-trusted-publishing Decision are
  still present after the PyPI edits : ☐ (PRESERVED / REGRESSION)

## Residue

- The external trusted-publisher registration is an out-of-repo follow-up recorded in `plan.md`'s
  Open questions; it is not an obligation of this task and need not be done for the spec edits to be
  correct.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
