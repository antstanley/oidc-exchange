# Done Certificate — Task 02: build-python + publish-pypi workflow

**Task:** [02-publish_pypi_workflow.md](02-publish_pypi_workflow.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-06-30

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
  - *Status:* ☑ SATISFIED — `release.yml:287-333`. Build step (`:322`) `uses:
    PyO3/maturin-action@e83996d129638aa358a18fbd1dfb82f0b0fb5d3b # v1.51.0` (40-hex pin), driven by
    a matrix: Linux `x86_64-unknown-linux-gnu` (`:295`) and `aarch64-unknown-linux-gnu` (`:300`)
    both set `manylinux: "2_28"`, `command: build`, `args: --release --out dist`; macOS
    `aarch64-apple-darwin` (`:306`) and Windows `x86_64-pc-windows-msvc` (`:310`) have NO
    `manylinux` key (native build), `command: build`, `args: --release --out dist`; a dedicated
    sdist leg (`:315`, ubuntu, no target) runs `command: sdist`, `args: --out dist`. Upload (`:330`)
    `path: bindings/python/dist/*`. The old bare `uvx maturin build --release --target` step is
    deleted (jj diff) and `grep` for it returns nothing — no plain `linux_x86_64` wheel produced.

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
  - *Status:* ☑ SATISFIED — `release.yml:335-355`. `needs: [build-python]` (`:337`),
    `permissions: { id-token: write }` (`:340-341`), `environment: pypi` (`:342`). Download (`:346`)
    `pattern: wheels-*`, `path: dist`, `merge-multiple: true`. Publish (`:353`) `uses:
    pypa/gh-action-pypi-publish@cef221092ed1bacb1cc03d23a2d87d1d172e277b # v1.14.0` with `with:
    packages-dir: dist/` ONLY — no `password`, no env. `yaml.safe_load` → YAML OK. `id-token: write`
    appears only at `:341` (publish-pypi) and `:211` (pre-existing publish-npm); `build-python` has
    NO `permissions` block → build code never holds the publish credential.

- **O3 — No PYPI_TOKEN / TWINE / twine remains anywhere in the workflow.**
  - *Claim:* no PyPI upload references `PYPI_TOKEN`, `TWINE_PASSWORD`/`TWINE_USERNAME`, or
    `twine upload`; authentication is OIDC trusted publishing.
  - *Evidence to collect:* `grep -nE 'PYPI_TOKEN|TWINE_|twine' .github/workflows/release.yml` —
    expect no matches (negative space).
  - *Status:* ☑ SATISFIED — `grep -nE 'PYPI_TOKEN|TWINE_|twine' .github/workflows/release.yml`
    returns no matches (exit 1). Only residual mention is the comment at `:352` ("No password/token:
    authentication is PyPI Trusted Publishing via OIDC"), which contains none of the patterns.

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
  - *Status:* ☑ SATISFIED — `grep 'uses:' | grep -vE '@[0-9a-f]{40} '` returns empty: every `uses:`
    in the file (incl. the two new pins at `:322`/`:353`) is a 40-hex SHA with a version comment. The
    two new pins were resolved against their claimed tags via `gh api`:
    `PyO3/maturin-action` v1.51.0 → `e83996d129638aa358a18fbd1dfb82f0b0fb5d3b` (matches `:322`);
    `pypa/gh-action-pypi-publish` v1.14.0 → `cef221092ed1bacb1cc03d23a2d87d1d172e277b` (matches `:353`).
    `create-release.needs` (`:359-364`) = validate, build-binaries, build-docker, **publish-npm**,
    **publish-pypi** — `publish-python` absent (whole-file `grep` returns nothing), npm dependency
    preserved. All `needs:` (`:53,125,169,207,289,337,359`) resolve to existing jobs (validate,
    build-binaries, build-docker, build-nodejs, publish-npm, build-python, publish-pypi,
    create-release) — no dangling dependency.

- **O5 — Meets the repo definition of done for what the task touches.**
  - *Claim:* the workflow is valid YAML and passes `actionlint` if available; no language test suite
    is affected (CI/packaging only).
  - *Evidence to collect:* per `.specs/development-guidelines.md` §Definition of done (CI is the
    enforcement gate), run `actionlint .github/workflows/release.yml` if present — expect clean (or
    record its absence and fall back to a YAML parse); confirm via `jj diff --name-only` that only
    `.github/workflows/release.yml` changed, so no Rust/TS/Python source is touched.
  - *Status:* ☑ SATISFIED (with environment note) — `actionlint` is absent in this environment
    (`which actionlint` → not found): UNVERIFIED-environment, not a defect. Fell back per the
    evidence instruction to `python3 -c "import yaml; yaml.safe_load(...)"` → YAML OK.
    `jj diff --name-only` → only `.github/workflows/release.yml` changed, so no Rust/TS/Python
    source is touched.

- **O6 — Reviewable: a reviewer confirms the manylinux/abi3 build + trusted publishing (Reviewable).**
  - *Claim:* a reviewer reads `build-python` and `publish-pypi` and confirms the manylinux_2_28
    Linux wheels, the macOS/Windows native wheels, the sdist, the build/publish separation (only
    `publish-pypi` holds `id-token: write`), the `pypi` Environment gate, the
    `pypa/gh-action-pypi-publish` upload with no token, and the SHA pins — and confirms
    `create-release` still resolves its `needs`.
  - *Evidence to collect:* walk the `build-python` and `publish-pypi` jobs top to bottom in
    `release.yml`; cross-check against O1–O4 evidence; confirm the manylinux+sdist build, no token,
    SHA pins, and a resolvable `create-release.needs`.
  - *Status:* ☑ SATISFIED — both jobs were walked top to bottom (`:287-355`) and cross-checked
    against O1–O4: manylinux_2_28 Linux + native macOS/Windows + sdist (O1), build/publish
    separation with `id-token: write` only on `publish-pypi` (O2), `pypi` Environment gate (O2),
    `pypa/gh-action-pypi-publish` upload with no token (O2/O3), all 40-hex SHA pins (O4), and a
    fully-resolvable `create-release.needs` (O4). Artifact wiring traced: `build-python` upload name
    `wheels-${{ matrix.target || matrix.command }}` produces five names (4 wheel triples + `sdist`),
    all matched by `publish-pypi`'s download `pattern: wheels-*` with `merge-multiple: true`.

## Regression check

- `create-release` (`.github/workflows/release.yml`) `needs` the PyPI publish job. Trace: after the
  rename, `create-release.needs` lists `publish-pypi` and every named job exists :
  ☑ PRESERVED — `:364` lists `publish-pypi`; all five `needs` entries resolve to existing jobs.
- The npm plan's `create-release.needs` edit must survive. Trace: the npm publish job
  (`publish-nodejs`/`publish-npm`) is still listed in `create-release.needs` after this task :
  ☑ PRESERVED — `publish-npm` present at `:363`; `publish-npm` job (`:205`) and its `id-token: write`
  (`:211`) untouched (not in jj diff).
- The `build-python` matrix feeds `publish-pypi` via the uploaded artifacts. Trace: `build-python`
  uploads `dist/*` under the artifact name `publish-pypi`'s download step consumes :
  ☑ PRESERVED — upload `name: wheels-${{ matrix.target || matrix.command }}` (`:332`),
  `path: bindings/python/dist/*` (`:333`); download `pattern: wheels-*` (`:348`) +
  `merge-multiple: true` → all wheels and the sdist reach the publish step's `dist/`.

## Residue

- Whether the sdist is its own job or a matrix leg, and whether a TestPyPI dry-run is added, are the
  implementer's choices and not obligations, provided O1–O3 hold. (Implementer chose a dedicated
  sdist matrix leg; O1–O3 hold.)
- Live behaviour — that trusted publishing actually authenticates against PyPI, and that the
  manylinux/cross builds succeed on the runners — requires the out-of-repo trusted-publisher
  registration and a real tag push; a headless validator cannot drive it. The *workflow wiring* is
  verified statically (above); the live publish + cross-build success are surfaced for manual
  confirmation rather than failing the task. `actionlint` absent in this environment
  (UNVERIFIED-environment) — YAML parse used as the fallback gate.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: `build-python` is a `PyO3/maturin-action` matrix (manylinux_2_28 ×2 Linux, native
macOS+Windows, sdist, all `--out dist` → `bindings/python/dist/*`) and `publish-pypi` is OIDC
trusted publishing with correct gating and no token; all six obligations are SATISFIED with named
evidence, both new SHA pins verified against their tags via `gh api`, every regression PRESERVED, and
only `release.yml` changed — with live publish/cross-build and absent `actionlint` recorded as
residue, not defects.
