# Done Certificate — Task 02: Python strict type checker (mypy)

**Task:** [02-python_type_checker.md](02-python_type_checker.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-06-29

> This certificate is a verification protocol for Task 02. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 02) ≡ every obligation O1…O5 below holds, each backed by the evidence the obligation
names (a command result or a config location) — not by assertion.

## Premises

- **P1 — Goal.** The task produces strict-mode `mypy` configured in `bindings/python` that passes
  clean over the package sources, plus a `python-typecheck` step in CI.
- **P2 — Obligations.** Done iff O1…O5 all hold. One Oi per definition-of-done item, in DoD
  order; O5 is the `Reviewable:` item.
- **P3 — Invariants.** Must not break the existing Python binding: `uv run pytest tests/` and the
  ruff format/lint gates must still pass, and the native-extension import path
  (`oidc_exchange._oidc_exchange`) must keep working.

## Obligations

- **O1 — `uv run mypy python` passes in strict mode.**
  - *Claim:* running mypy over the package source root `python` reports no issues.
  - *Evidence to collect:* in `bindings/python`, run `uv run mypy python` — expect exit 0 and a
    `Success: no issues found` line. Capture the command output.
  - *Checks:* confirm strict mode is actually in force (the run reflects `strict = true`), not a
    lax default — e.g. a deliberately untyped function would be flagged.
  - *Evidence collected:* in `bindings/python`, `uv run mypy python` →
    `Success: no issues found in 3 source files` (exit 0). The 3 files are `__init__.pyi`,
    `_asgi.py`, `_wsgi.py` — the hand-curated stub shadows `__init__.py` for the `oidc_exchange`
    module, so the strictly-checked pure-Python sources are the adapters.
  - *Check result:* strict mode confirmed in force — a probe `def untyped(x): return x + 1` run
    against the project's `pyproject.toml` config errored `[no-untyped-def]` (exit 1), proving
    `strict = true` is applied, not a lax default.
  - *Status:* ☑ SATISFIED

- **O2 — pyproject.toml carries mypy (dev) and a scoped strict config.**
  - *Claim:* `bindings/python/pyproject.toml` adds `mypy` to the `dev` group and a `[tool.mypy]`
    section with `strict = true`; any import override is narrowly scoped and justified.
  - *Evidence to collect:* read `bindings/python/pyproject.toml`; confirm `mypy` in
    `[dependency-groups].dev`, a `[tool.mypy]` block with `strict = true`, and that any override
    (e.g. `ignore_missing_imports` for `oidc_exchange._oidc_exchange`) targets only the native
    module and carries an inline reason.
  - *Checks:* confirm the override does not blanket-disable type checking of the pure-Python
    sources (negative-space — `_asgi.py`/`_wsgi.py`/`__init__.py` are still strictly checked).
  - *Evidence collected:* read `bindings/python/pyproject.toml` —
    `mypy>=2.1.0` is in `[dependency-groups].dev` (line 28); `[tool.mypy]` has `strict = true`,
    `python_version = "3.10"`, `files = ["python"]` (lines 43–46); the single override
    `[[tool.mypy.overrides]]` targets `module = ["oidc_exchange._oidc_exchange"]` with
    `ignore_missing_imports = true` and a 5-line inline justification (lines 48–55).
  - *Check result:* override is narrowly scoped to the native module only and does not disable
    checking of pure-Python sources — `_asgi.py` and `_wsgi.py` appear among mypy's 3 strictly
    checked files. Nuance: `__init__.py`'s body is type-only-shadowed by `__init__.pyi`, so its
    delegations are not strict-checked, but the public surface is typed via the stub and the
    substantive adapter logic IS strictly checked — the override itself is not what excludes it.
  - *Status:* ☑ SATISFIED

- **O3 — CI has a `python-typecheck` step positioned after the build.**
  - *Claim:* `.github/workflows/ci.yml` runs `uv run mypy` over `bindings/python` in the
    `python-test` job, after the extension module is built/available.
  - *Evidence to collect:* read `.github/workflows/ci.yml`; confirm a step named for type-checking
    runs `uv run mypy` with `working-directory: bindings/python`, ordered after `uv sync` and the
    `maturin develop` build step so the module import resolves.
  - *Evidence collected:* read `.github/workflows/ci.yml` — in the `python-test` job the steps run
    in order: `Install dependencies` → `uv sync` (l.96), `Lint` (l.97), `Check formatting` (l.100),
    `Build Python module` → `uv run maturin develop` (l.105), **`Type-check`** with
    `working-directory: bindings/python` → `uv run mypy python` (l.106–108), `Run tests` (l.109).
    The type-check step is positioned after both `uv sync` and the `maturin develop` build, so the
    extension module is available when mypy runs.
  - *Status:* ☑ SATISFIED

- **O4 — Meets the repo definition of done for Python.**
  - *Claim:* the Python format/lint/test gates still pass after the annotation changes.
  - *Evidence to collect:* in `bindings/python`, run `uv run ruff format --check .`, `uv run ruff
    check .`, and `uv run pytest tests/` (the commands from `.specs/development-guidelines.md`
    §Definition of done) — expect all clean/passing. Capture outputs.
  - *Evidence collected:* in `bindings/python` — `uv run ruff format --check .` →
    `7 files already formatted` (exit 0); `uv run ruff check .` → `All checks passed!` (exit 0);
    `uv run pytest tests/` → `11 passed in 0.07s` (exit 0). All Python gates clean after the
    annotation changes.
  - *Status:* ☑ SATISFIED

- **O5 — Reviewable: a reviewer runs mypy and reads the CI step (Reviewable).**
  - *Claim:* a reviewer can run `uv run mypy python` in `bindings/python` and see it pass, and read
    the new CI step.
  - *Evidence to collect:* run `uv run mypy python` and observe the success output; read the
    `python-typecheck` step in `.github/workflows/ci.yml` and confirm it would run the same gate in
    CI.
  - *Evidence collected:* exercised, not assumed — `uv run mypy python` was run in `bindings/python`
    and observed to pass (`Success: no issues found in 3 source files`); the `Type-check` step in
    `.github/workflows/ci.yml` (l.106–108) runs the identical `uv run mypy python` with
    `working-directory: bindings/python`, so CI enforces the same gate a reviewer runs locally.
  - *Status:* ☑ SATISFIED

## Regression check

- The task touches `bindings/python` sources to satisfy strict typing. Trace one downstream caller:
  the `tests/` (e.g. `test_handle_request.py`, `test_asgi.py`) import the package and exercise it —
  after the annotation changes, `uv run pytest tests/` still passes : ☑ PRESERVED — `uv run pytest
  tests/` reports `11 passed in 0.07s`. The annotations are purely additive (runtime signatures
  unchanged), so `test_asgi.py`/`test_wsgi.py`/`test_handle_request.py`, which import the package and
  exercise `asgi_app()`/`wsgi_app()`/`handle_request*`, all pass; the native `oidc_exchange._oidc_exchange`
  import path keeps working (the `.so` loads). ruff format/lint also remain clean.

## Residue

- Whether to extend mypy to `tests/` is deferred (task Open question); not an obligation here.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O5 are all SATISFIED with collected evidence (mypy clean and strict-confirmed by a
`[no-untyped-def]` probe, scoped override + dev-group dep in pyproject, CI `Type-check` step after the
maturin build, ruff/pytest clean) and the `tests/` regression caller is PRESERVED (11 passed); the
`.pyi`-shadows-`.py` nuance leaves `__init__.py`'s thin delegations unchecked but the public surface is
typed via the stub and the adapters are strictly checked, so the DoD holds.
