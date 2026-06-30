# Done Certificate — Task 02: build-python + publish-pypi workflow

**Task:** [02-publish_pypi_workflow.md](02-publish_pypi_workflow.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-06-30 — unverified   <!-- validator sets: Validated YYYY-MM-DD -->

> This certificate is a verification protocol for Task 02. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 02) ≡ every obligation O1…O6 below holds, each backed by the evidence the obligation
names (a YAML location, a grep result, or a parse) — not by assertion.

## Premises

- **P1 — Goal.** The task rebuilds `build-python` in `.github/workflows/release.yml` as a
  `PyO3/maturin-action` matrix producing `manylinux_2_28` abi3 wheels (Linux) + native wheels
  (macOS/Windows) + an sdist, and replaces `publish-python` with a `publish-pypi` job that builds
  nothing (it `needs: [build-python]`), holds `id-token: write`, runs in the `pypi` Environment,
  and uploads with `pypa/gh-action-pypi-publish` under PyPI Trusted Publishing — no `PYPI_TOKEN` —
  while SHA-pinning every action and fixing `create-release`'s `needs`.
- **P2 — Obligations.** Done iff O1…O6 all hold. One Oi per definition-of-done item, in DoD order;
  O6 is the `Reviewable:` item.
- **P3 — Invariants.** Must not break the rest of the release DAG: `validate`, `build-binaries`,
  `build-docker`, `build-nodejs`/`publish-nodejs` (or `publish-npm` if the npm plan landed), and
  `create-release` must still resolve their `needs`, and the npm publish dependency in
  `create-release.needs` must be preserved (not reverted by this edit).

## Obligations

- **O1 — build-python builds manylinux + native wheels and an sdist.**
  - *Claim:* `build-python` uses `PyO3/maturin-action` with `manylinux: 2_28` for both Linux
    targets and native builds for macOS/Windows, all with `--out dist`; an sdist is built with
    `maturin sdist`; wheels + sdist upload as artifacts.
  - *Evidence to collect:* read the `build-python` job in `.github/workflows/release.yml`; confirm
    the matrix uses `PyO3/maturin-action` (SHA-pinned), that the two Linux legs set
    `manylinux: 2_28` and `target:` the gnu triples, that macOS/Windows legs run native maturin
    builds, that one leg/step runs `maturin sdist` (action `command: sdist` or `uvx maturin sdist`),
    and that artifacts upload `dist/*`. Confirm no bare `uvx maturin build --release --target` on a
    plain `ubuntu-latest` remains.
  - *Checks:* negative space — a `build-python` that still emits a plain `linux_x86_64` wheel (no
    manylinux) is a defect; a missing sdist is a defect.
  - *Status:* ☐ unverified

- **O2 — publish-pypi exists with the correct gating and trusted-publishing upload.**
  - *Claim:* `publish-pypi` declares `needs: [build-python]`, `permissions: { id-token: write }`,
    and `environment: pypi`; it downloads artifacts into `dist/` and uploads with
    `pypa/gh-action-pypi-publish` and no `with: password`.
  - *Evidence to collect:* read the `publish-pypi` job; confirm the `needs`, `permissions`
    (`id-token: write`), and `environment: pypi` keys; confirm the upload step is
    `uses: pypa/gh-action-pypi-publish@<sha>` with no `password`/`TWINE_*` and `packages-dir`
    pointing at the downloaded `dist/`. Validate the file parses (actionlint or a YAML load).
  - *Checks:* confirm `id-token: write` lives **only** on `publish-pypi` (not on `build-python`),
    so build code never holds the publish credential.
  - *Status:* ☐ unverified

- **O3 — No PYPI_TOKEN / TWINE / twine remains anywhere in the workflow.**
  - *Claim:* no PyPI upload references `PYPI_TOKEN`, `TWINE_PASSWORD`/`TWINE_USERNAME`, or
    `twine upload`; authentication is OIDC trusted publishing.
  - *Evidence to collect:* `grep -nE 'PYPI_TOKEN|TWINE_|twine' .github/workflows/release.yml` —
    expect no matches (negative space).
  - *Status:* ☐ unverified

- **O4 — Every uses: is SHA-pinned and create-release needs publish-pypi without reverting npm.**
  - *Claim:* every `uses:` in the touched jobs is pinned to a full-length commit SHA; `create-release`'s
    `needs` references `publish-pypi` (not the removed `publish-python`) and still lists the npm
    publish job.
  - *Evidence to collect:* `grep -nE 'uses:' .github/workflows/release.yml` for the `build-python`
    and `publish-pypi` jobs; confirm each ref is a 40-hex-char SHA with a `# vX` comment. Read
    `create-release.needs` and confirm `publish-pypi` appears, `publish-python` does not, and the
    npm publish job (`publish-nodejs` or `publish-npm`) is still listed. `grep -n 'publish-python'
    .github/workflows/release.yml` — expect no matches.
  - *Checks:* trace every `needs:` list in the file — confirm none references a job name that no
    longer exists (no dangling dependency after the rename).
  - *Status:* ☐ unverified

- **O5 — Meets the repo definition of done for what the task touches.**
  - *Claim:* the workflow is valid YAML and passes `actionlint` if available; no language test suite
    is affected (CI/packaging only).
  - *Evidence to collect:* per `.specs/development-guidelines.md` §Definition of done (CI is the
    enforcement gate), run `actionlint .github/workflows/release.yml` if present — expect clean (or
    record its absence and fall back to a YAML parse); confirm via `jj diff --name-only` that only
    `.github/workflows/release.yml` changed, so no Rust/TS/Python source is touched.
  - *Status:* ☐ unverified

- **O6 — Reviewable: a reviewer confirms the manylinux/abi3 build + trusted publishing (Reviewable).**
  - *Claim:* a reviewer reads `build-python` and `publish-pypi` and confirms the manylinux_2_28
    Linux wheels, the macOS/Windows native wheels, the sdist, the build/publish separation (only
    `publish-pypi` holds `id-token: write`), the `pypi` Environment gate, the
    `pypa/gh-action-pypi-publish` upload with no token, and the SHA pins — and confirms
    `create-release` still resolves its `needs`.
  - *Evidence to collect:* walk the `build-python` and `publish-pypi` jobs top to bottom in
    `release.yml`; cross-check against O1–O4 evidence; confirm the manylinux+sdist build, no token,
    SHA pins, and a resolvable `create-release.needs`.
  - *Status:* ☐ unverified

## Regression check

- `create-release` (`.github/workflows/release.yml`) `needs` the PyPI publish job. Trace: after the
  rename, `create-release.needs` lists `publish-pypi` and every named job exists : ☐ (PRESERVED / REGRESSION)
- The npm plan's `create-release.needs` edit must survive. Trace: the npm publish job
  (`publish-nodejs`/`publish-npm`) is still listed in `create-release.needs` after this task :
  ☐ (PRESERVED / REGRESSION)
- The `build-python` matrix feeds `publish-pypi` via the uploaded artifacts. Trace: `build-python`
  uploads `dist/*` under the artifact name `publish-pypi`'s download step consumes : ☐ (PRESERVED / REGRESSION)

## Residue

- Whether the sdist is its own job or a matrix leg, and whether a TestPyPI dry-run is added, are the
  implementer's choices and not obligations, provided O1–O3 hold.
- Live behaviour — that trusted publishing actually authenticates against PyPI, and that the
  manylinux/cross builds succeed on the runners — requires the out-of-repo trusted-publisher
  registration and a real tag push; a headless validator cannot drive it. Verify the *workflow
  wiring* statically; surface the live publish for manual confirmation rather than failing the task
  on it.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
