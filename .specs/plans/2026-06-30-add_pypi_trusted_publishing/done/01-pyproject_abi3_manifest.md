# Task 01 — pyproject abi3 manifest

**Plan:** [plan.md](../plan.md) · **Certificate:** [01-pyproject_abi3_manifest-certificate.md](01-pyproject_abi3_manifest-certificate.md)

**Implements:** [.specs/changes/merged/2026-06-29-add_pypi_trusted_publishing.md](../../../changes/merged/2026-06-29-add_pypi_trusted_publishing.md) §Implementation notes 1 (set the maturin `abi3-py310` feature; confirm `Cargo.toml` parity); satisfies the [.specs/bindings/specs/03-python.md](../../../bindings/specs/03-python.md) §Distribution `abi3-py310` fact recorded in task 03.
**Depends on:** —
**Produces:** `bindings/python/pyproject.toml` `[tool.maturin]` declares `features = ["pyo3/extension-module", "pyo3/abi3-py310"]` so the abi3 contract is explicit at the maturin level (not only in `Cargo.toml`), and a local `maturin build` emits a single `cp310-abi3` wheel; `bindings/python/Cargo.toml`'s `pyo3` features are confirmed to enable the same `abi3-py310` (they already do); `pyproject.toml` `version` is unchanged so version parity with `Cargo.toml` / `bindings/nodejs/package.json` holds.
**Pointers:** `bindings/python/pyproject.toml:31`-`:34` (`[tool.maturin]` block — `features`, `python-source`, `module-name`); `bindings/python/Cargo.toml:12` (`pyo3 = { version = "0.22", features = ["extension-module", "abi3-py310"] }` — already abi3); `bindings/python/pyproject.toml:7` (`version` — must stay parity-aligned).

## Steps

- [ ] Set `bindings/python/pyproject.toml` `[tool.maturin] features` to `["pyo3/extension-module", "pyo3/abi3-py310"]`, making the abi3 contract explicit at the maturin level rather than implicit in `Cargo.toml` only.
- [ ] Confirm `bindings/python/Cargo.toml`'s `pyo3` dependency enables `abi3-py310` (it does at `:12`); leave it as-is so the maturin feature and the crate feature agree (no divergence, no double-enable conflict).
- [ ] Leave `pyproject.toml` `version`, `requires-python = ">=3.10"`, and the `Programming Language :: Python :: 3.1x` classifiers untouched — the abi3 single-wheel claim already matches them; this task only locks in the feature.
- [ ] Build locally to verify: run `uvx maturin build --release` (or `uv run maturin build --release`) in `bindings/python` and read the emitted wheel filename in `target/wheels/`, confirming it carries the `cp310-abi3` ABI tag (not a bare `cp310`).
- [ ] Run the Python format/lint gate (`uv run ruff format --check .`, `uv run ruff check .`) to confirm the manifest edit leaves the package clean; no `.py`/`.rs` source changed so `pyright`/`pytest` are unaffected (run them if a fast check is available and report the result).

## Definition of done

- [ ] `bindings/python/pyproject.toml` `[tool.maturin] features` lists both `pyo3/extension-module` and `pyo3/abi3-py310` (verify by reading the file / parsing the TOML); `Cargo.toml` `pyo3` features still include `abi3-py310` so the two agree.
- [ ] A local `maturin build --release` emits exactly one wheel whose filename carries the `cp310-abi3` tag (e.g. `oidc_exchange-<version>-cp310-abi3-<platform>.whl`) — negative space: a bare `cp310-cp310` (non-abi3) tag is a defect, as is more than one wheel for the host interpreter.
- [ ] Version parity holds: `pyproject.toml` `version` is unchanged and still equals `Cargo.toml` `workspace.package.version` and `bindings/nodejs/package.json` `version`, so the release `validate` job is not regressed.
- [ ] Meets the repo definition of done for the languages touched: `uv run ruff format --check .` and `uv run ruff check .` pass in `bindings/python`; no Python/Rust source changed so `pyright`/`pytest` are unaffected (report if run).
- [ ] Reviewable: a reviewer reads the updated `[tool.maturin]` block, confirms it names `pyo3/abi3-py310` and agrees with `Cargo.toml`, and runs `maturin build` to confirm the emitted wheel is `cp310-abi3`.

## Open questions

- None at the task level. Whether the manylinux build needs the maturin feature at all (since `Cargo.toml` already enables abi3) is moot — making it explicit is belt-and-suspenders and the DoD verifies the resulting tag regardless of which layer enables it.
