# Task 02 — Python strict type checker (mypy)

**Plan:** [plan.md](../plan.md) · **Certificate:** [02-python_type_checker-certificate.md](02-python_type_checker-certificate.md)

**Implements:** [.specs/changes/2026-06-24-add_local_enforcement_gates.md](../../../changes/2026-06-24-add_local_enforcement_gates.md) §Implementation notes 2 (Python type checker); enables the [.specs/development-guidelines.md](../../../development-guidelines.md) §Toolchain mypy row added in task 03.
**Depends on:** —
**Produces:** strict-mode `mypy` configured in `bindings/python/pyproject.toml` (added to the dev dependency group and a `[tool.mypy]` section), passing clean over the Python sources, plus a `python-typecheck` step in `.github/workflows/ci.yml`.
**Pointers:** `bindings/python/pyproject.toml` (`[dependency-groups].dev`, add `[tool.mypy]`); Python sources at `bindings/python/python/oidc_exchange/` (`__init__.py`, `_asgi.py`, `_wsgi.py`, plus `__init__.pyi`/`py.typed`); `.github/workflows/ci.yml` `python-test` job (lines 83–108).

## Steps

- [ ] Add `mypy` to the `dev` dependency group in `bindings/python/pyproject.toml`.
- [ ] Add a `[tool.mypy]` section enabling strict mode (`strict = true`), targeting `python` (the package source root), `python_version = "3.10"`; add a narrowly-scoped override for the native extension module `oidc_exchange._oidc_exchange` (`ignore_missing_imports = true`) since it is a compiled `.so` with a hand-curated `.pyi`, so the override is justified inline.
- [ ] Run `uv run mypy python` inside `bindings/python` and resolve every strict-mode finding by adding precise type annotations (not blanket `# type: ignore`); where an ignore is unavoidable, scope it to a specific error code with a one-line reason.
- [ ] Add a `python-typecheck` step to the `python-test` job in `.github/workflows/ci.yml` (after dependency install / module build, since the sources import the built extension) running `uv run mypy python`.
- [ ] Confirm `uv run ruff format --check .`, `uv run ruff check .`, and `uv run pytest tests/` still pass after the annotation changes.

## Definition of done

- [ ] `uv run mypy python` exits 0 over `bindings/python` in strict mode (verify by running it and capturing the "Success: no issues found" line).
- [ ] `bindings/python/pyproject.toml` carries `mypy` in the dev group and a `[tool.mypy]` strict section; any import override is scoped and justified (negative-space: the native-module import does not silently disable type-checking of the pure-Python sources).
- [ ] `.github/workflows/ci.yml` has a `python-typecheck` step that runs `uv run mypy` over `bindings/python` and is positioned so the extension module is available.
- [ ] Meets the repo definition of done for Python: `uv run ruff format --check .`, `uv run ruff check .`, and `uv run pytest tests/` all pass; every public function/class touched keeps its annotations and docstrings.
- [ ] Reviewable: a reviewer runs `uv run mypy python` in `bindings/python` and sees it pass, and reads the new CI step.

## Open questions

- Whether to type-check `tests/` as well as the package source. The task scopes mypy to the
  shipped package (`python`) to keep the gate focused on the public surface; widening to `tests/`
  is deferred.
