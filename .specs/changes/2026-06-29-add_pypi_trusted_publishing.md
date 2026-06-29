# Change: Publish `oidc-exchange` wheels to PyPI via trusted publishing

**Status:** Proposed · **Date:** 2026-06-29 · **Owner:** Ant Stanley · **Target:** bindings/python, .github/workflows

Add a dedicated PyPI publish job to `release.yml` that builds `abi3` manylinux / macOS / Windows
wheels plus an sdist for `oidc-exchange` and uploads them with **PyPI Trusted Publishing** (GitHub
OIDC, no `PYPI_TOKEN`).

---

## Motivation

The canonical spec describes wheels the build does not produce.
[`03-python.md`](../bindings/specs/03-python.md) and
[`05-distribution.md`](../bindings/specs/05-distribution.md) state "`abi3` stable ABI targeting
Python 3.10+ — one wheel per platform works across 3.10–3.13" on
"`manylinux_2_28_{x86_64,aarch64}`, `win_amd64`, `macosx_11_0_arm64`". The code does neither:
`bindings/python/pyproject.toml` enables only `pyo3/extension-module` (no `abi3`/`abi3-py310`
feature), and `build-python` runs `maturin build --release --target <triple>` on a bare
ubuntu-latest runner with Python 3.10 — producing a single, version-specific `cp310`, non-`abi3`,
non-manylinux wheel per platform. PyPI rejects a plain `linux_x86_64` wheel, and the `cp310`-only
tag leaves 3.11–3.13 users to compile from source (which needs a Rust toolchain).

Separately, `publish-python` uploads with `twine` and a long-lived `PYPI_TOKEN` secret. PyPI
Trusted Publishing issues short-lived OIDC credentials per run via
`pypa/gh-action-pypi-publish`, removing the stored secret.

---

## Affected spec pages

| Canonical page | Nature of change |
|---|---|
| [`.specs/bindings/specs/05-distribution.md`](../bindings/specs/05-distribution.md) → Release pipeline | `build-python` builds `abi3` manylinux wheels + sdist; `publish-pypi` uses trusted publishing |
| [`.specs/bindings/specs/05-distribution.md`](../bindings/specs/05-distribution.md) → Assumptions / Decisions | Replace "PyPI credentials configured as secrets" with OIDC trusted publishing |
| [`.specs/bindings/specs/03-python.md`](../bindings/specs/03-python.md) → Distribution | Note `pyproject.toml` enables the `abi3-py310` feature; manylinux build via maturin |

---

## Proposed changes

### `.specs/bindings/specs/05-distribution.md` → Release pipeline (Modify)

Replace the `build-python` + `publish-python` description with:

> `build-python` (matrix per platform) builds an `abi3` wheel with maturin in a manylinux_2_28
> container (`PyO3/maturin-action`, `manylinux: 2_28` for the Linux targets), and one job also
> builds the sdist (`maturin sdist`); all wheels and the sdist upload as artifacts. Because the
> extension is built against the Python 3.10 stable ABI, one wheel per platform covers 3.10–3.13.
> `publish-pypi` then runs as a separate job — it needs `build-python`, declares
> `permissions: { id-token: write }`, runs in the `pypi` GitHub Environment, downloads every
> wheel and the sdist, and uploads them with `pypa/gh-action-pypi-publish`. Authentication is PyPI
> Trusted Publishing — no `PYPI_TOKEN`. Every action is pinned to a full-length commit SHA.

### `.specs/bindings/specs/05-distribution.md` → Assumptions / Decisions (Modify)

Drop PyPI from the "credentials configured as repository secrets" assumption and add a Decision:

> - *PyPI trusted publishing.* **`oidc-exchange` wheels publish via GitHub OIDC, not a stored
>   `PYPI_TOKEN`.** `pypa/gh-action-pypi-publish` exchanges the workflow's OIDC token for a
>   short-lived upload credential; the project is registered as a trusted publisher for
>   `antstanley/oidc-exchange` / `release.yml` on PyPI.

### `.specs/bindings/specs/03-python.md` → Distribution (Modify)

Replace the Distribution paragraph with:

> maturin, `abi3` stable ABI targeting Python 3.10+ — `pyproject.toml` enables the
> `pyo3/abi3-py310` feature so one wheel per platform works across 3.10–3.13. Linux wheels are
> built in a `manylinux_2_28` container; platforms: `manylinux_2_28_{x86_64,aarch64}`,
> `win_amd64`, `macosx_11_0_arm64`. An sdist is published alongside the wheels. See
> [05-distribution.md](05-distribution.md).

---

## Type changes

None. Packaging and CI only; no Python API or domain changes.

---

## Implementation notes

1. `bindings/python/pyproject.toml`: set the pyo3 `abi3-py310` feature so maturin emits an `abi3`
   wheel, e.g. `[tool.maturin] features = ["pyo3/extension-module", "pyo3/abi3-py310"]`. Confirm
   `Cargo.toml` for `oidc-exchange-python` enables the same feature if it pins pyo3 features
   directly.
2. `release.yml`: replace `build-python` with a `PyO3/maturin-action` matrix:
   - Linux `x86_64`/`aarch64`: `command: build`, `args: --release --out dist`,
     `manylinux: 2_28`, `target: <triple>`.
   - macOS `aarch64`, Windows `x86_64`: native maturin build, `--out dist`.
   - One job runs `maturin sdist --out dist`.
   - Upload `dist/*` artifacts.
3. Replace `publish-python` with `publish-pypi`: `needs: [build-python]`,
   `permissions: { id-token: write }`, `environment: pypi`. Download artifacts to `dist/`, then
   `uses: pypa/gh-action-pypi-publish@<sha>` (no `with: password`). Remove the `twine`/`PYPI_TOKEN`
   path.
4. One-time, outside the repo: register `oidc-exchange` as a trusted publisher on PyPI bound to
   `antstanley/oidc-exchange`, `release.yml`, and the `pypi` environment; create the `pypi` GitHub
   Environment restricted to the release tag. Remove the `PYPI_TOKEN` secret once verified.

References: maturin user guide (abi3, manylinux, `maturin-action`); PyPI Trusted Publishing;
`pypa/gh-action-pypi-publish`.

---

## Merge plan

1. Apply each `Proposed changes` block to its canonical page; bump each page's `**Date:**`.
2. Update the `05-distribution.md` Assumptions/Decisions per the block above.
3. No schema change.
4. Flip `**Status:**` to `Merged`, stamp `**Merged:**`, move to `.specs/changes/merged/`.
5. Update `.specs/README.md`.

---

## Assumptions and open questions

### Assumptions

- The `oidc-exchange` project name is owned (or available) on PyPI so it can be registered as a
  trusted publisher.
- The four target platforms in the canonical spec are the supported set; no 32-bit or musllinux
  wheels are in scope.

### Decisions

- *`abi3-py310` single wheel.* **One stable-ABI wheel per platform spans 3.10–3.13.** Matches the
  abi3 claim the spec already makes and keeps the build matrix to one Python per platform.
- *Separate build and publish jobs.* **Only `publish-pypi` holds `id-token: write`.** Build code
  never sees the publish credential, mirroring the npm change.

### Open questions

- Whether to publish to TestPyPI first (a `testpypi` environment + dry-run) before the production
  upload is left to implementation.
- `aarch64` macOS and Windows `abi3` cross-builds are assumed to work under `maturin-action`/native
  runners; if a target needs `zig` or a self-hosted runner, that is resolved at implementation.
