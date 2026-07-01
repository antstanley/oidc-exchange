# Task 02 — build-python + publish-pypi workflow

**Plan:** [plan.md](../plan.md) · **Certificate:** [02-publish_pypi_workflow-certificate.md](02-publish_pypi_workflow-certificate.md)

**Implements:** [.specs/changes/merged/2026-06-29-add_pypi_trusted_publishing.md](../../../changes/merged/2026-06-29-add_pypi_trusted_publishing.md) §Implementation notes 2 & 3 (rebuild `build-python` as a manylinux maturin matrix + sdist; replace `publish-python` with `publish-pypi`); realises the [.specs/bindings/specs/05-distribution.md](../../../bindings/specs/05-distribution.md) §Release pipeline `build-python` → `publish-pypi` description recorded in task 03.
**Depends on:** 01
**Produces:** `.github/workflows/release.yml` rebuilds `build-python` as a `PyO3/maturin-action` matrix — Linux `x86_64`/`aarch64` with `manylinux: 2_28` and `args: --release --out dist`, macOS `aarch64` and Windows `x86_64` as native maturin builds (`--out dist`), and one leg (or step) running `maturin sdist --out dist` — uploading `dist/*` as artifacts; and replaces `publish-python` with a `publish-pypi` job declaring `needs: [build-python]`, `permissions: { id-token: write }`, `environment: pypi`, that downloads every wheel + the sdist into `dist/` and uploads them with `pypa/gh-action-pypi-publish` (no `with: password`, no `PYPI_TOKEN`) under PyPI Trusted Publishing; every `uses:` is SHA-pinned and `create-release`'s `needs` is updated from `publish-python` to `publish-pypi`.
**Pointers:** `.github/workflows/release.yml:238` (`build-python` matrix — rebuild on `maturin-action`), `:263`-`:265` (the bare `uvx maturin build` step to replace), `:266`-`:270` (artifact upload — repath to `dist/*`), `:272`-`:291` (`publish-python` with `uvx twine upload` + `PYPI_TOKEN` — replace), `:293`-`:301` (`create-release` `needs` list, `publish-python` at `:300`); `bindings/python/pyproject.toml` (maturin config the action reads).

## Steps

- [ ] Rebuild `build-python` as a `PyO3/maturin-action` matrix: for Linux `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu` set `manylinux: 2_28`, `command: build`, `args: --release --out dist`, `target: <triple>`; for `aarch64-apple-darwin` (macOS) and `x86_64-pc-windows-msvc` (Windows) run a native maturin build with `args: --release --out dist` (no manylinux). Pin `PyO3/maturin-action` to a full-length commit SHA.
- [ ] Add an sdist build — either a dedicated matrix leg or a separate job/step — running `maturin sdist --out dist` (via `maturin-action` `command: sdist` or `uvx maturin sdist`), so an sdist ships alongside the wheels.
- [ ] Upload `dist/*` (wheels and sdist) as artifacts under a `python-*` / `dist-*` name pattern the publish job can download with `merge-multiple: true`.
- [ ] Replace `publish-python` with a `publish-pypi` job: `needs: [build-python]`, `permissions: { id-token: write }`, `environment: pypi`, `runs-on: ubuntu-latest`. Download all build artifacts into `dist/`.
- [ ] Upload with `uses: pypa/gh-action-pypi-publish@<sha>` and **no** `with: password` (and no `TWINE_*`/`PYPI_TOKEN` env) — authentication is OIDC trusted publishing; point `packages-dir` at `dist/` if it is not the default. Remove the `uvx twine upload` step and every `PYPI_TOKEN` reference.
- [ ] Pin every `uses:` in the new/edited jobs to a full-length commit SHA (carry existing pinned SHAs forward); update `create-release`'s `needs` from `publish-python` to `publish-pypi`. Leave `create-release`'s npm dependency (`publish-nodejs`/`publish-npm`) intact so the npm plan's edit is not reverted.

## Definition of done

- [ ] `build-python` builds via `PyO3/maturin-action` with `manylinux: 2_28` for both Linux targets and native builds for macOS/Windows, all with `--out dist`; an sdist is built with `maturin sdist`; wheels + sdist upload as artifacts (verify by reading the job — a bare `linux_x86_64` wheel is no longer produced).
- [ ] `publish-pypi` exists with `needs: [build-python]`, `permissions: { id-token: write }`, and `environment: pypi`; it downloads the artifacts into `dist/` and uploads with `pypa/gh-action-pypi-publish` and no `with: password` (verify by reading the job).
- [ ] No `PYPI_TOKEN` / `TWINE_PASSWORD` / `twine upload` reference remains anywhere in `release.yml` — negative space: grep the file for `PYPI_TOKEN`, `TWINE_`, and `twine` returns nothing.
- [ ] Every `uses:` in the touched jobs is pinned to a full-length commit SHA (no floating `@v*`/branch ref), and `create-release`'s `needs` references `publish-pypi` (not the removed `publish-python`) while still listing the npm publish job, so the release DAG has no dangling dependency.
- [ ] Meets the repo definition of done for what the task touches: the workflow is valid YAML and passes `actionlint` if available (report its absence otherwise); the change is CI/packaging only, so no Rust/TS/Python test suite is affected.
- [ ] Reviewable: a reviewer reads the `build-python` and `publish-pypi` jobs and confirms the manylinux_2_28 Linux wheels, the macOS/Windows native wheels, the sdist, the build/publish separation (only `publish-pypi` holds `id-token: write`), the `pypi` Environment gate, the `pypa/gh-action-pypi-publish` upload with no token, the SHA pins — and confirms `create-release` still resolves its `needs`.

## Open questions

- Whether the sdist is built in its own job or as a leg of the `build-python` matrix is left to the implementer; either is acceptable provided exactly one sdist ships and it lands in the artifacts the publish job downloads.
- Whether to add a TestPyPI dry-run (a `testpypi` environment + a `repository-url` override on a non-tag trigger) before the production upload is optional polish, not required by the DoD.
