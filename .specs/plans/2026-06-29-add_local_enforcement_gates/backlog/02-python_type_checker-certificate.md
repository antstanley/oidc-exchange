# Done Certificate — Task 02: Python strict type checker (mypy)

**Task:** [02-python_type_checker.md](02-python_type_checker.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-06-29 — unverified   <!-- validator sets: Validated YYYY-MM-DD -->

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
  - *Status:* ☐ unverified

- **O2 — pyproject.toml carries mypy (dev) and a scoped strict config.**
  - *Claim:* `bindings/python/pyproject.toml` adds `mypy` to the `dev` group and a `[tool.mypy]`
    section with `strict = true`; any import override is narrowly scoped and justified.
  - *Evidence to collect:* read `bindings/python/pyproject.toml`; confirm `mypy` in
    `[dependency-groups].dev`, a `[tool.mypy]` block with `strict = true`, and that any override
    (e.g. `ignore_missing_imports` for `oidc_exchange._oidc_exchange`) targets only the native
    module and carries an inline reason.
  - *Checks:* confirm the override does not blanket-disable type checking of the pure-Python
    sources (negative-space — `_asgi.py`/`_wsgi.py`/`__init__.py` are still strictly checked).
  - *Status:* ☐ unverified

- **O3 — CI has a `python-typecheck` step positioned after the build.**
  - *Claim:* `.github/workflows/ci.yml` runs `uv run mypy` over `bindings/python` in the
    `python-test` job, after the extension module is built/available.
  - *Evidence to collect:* read `.github/workflows/ci.yml`; confirm a step named for type-checking
    runs `uv run mypy` with `working-directory: bindings/python`, ordered after `uv sync` and the
    `maturin develop` build step so the module import resolves.
  - *Status:* ☐ unverified

- **O4 — Meets the repo definition of done for Python.**
  - *Claim:* the Python format/lint/test gates still pass after the annotation changes.
  - *Evidence to collect:* in `bindings/python`, run `uv run ruff format --check .`, `uv run ruff
    check .`, and `uv run pytest tests/` (the commands from `.specs/development-guidelines.md`
    §Definition of done) — expect all clean/passing. Capture outputs.
  - *Status:* ☐ unverified

- **O5 — Reviewable: a reviewer runs mypy and reads the CI step (Reviewable).**
  - *Claim:* a reviewer can run `uv run mypy python` in `bindings/python` and see it pass, and read
    the new CI step.
  - *Evidence to collect:* run `uv run mypy python` and observe the success output; read the
    `python-typecheck` step in `.github/workflows/ci.yml` and confirm it would run the same gate in
    CI.
  - *Status:* ☐ unverified

## Regression check

- The task touches `bindings/python` sources to satisfy strict typing. Trace one downstream caller:
  the `tests/` (e.g. `test_handle_request.py`, `test_asgi.py`) import the package and exercise it —
  after the annotation changes, `uv run pytest tests/` still passes : ☐ (PRESERVED / REGRESSION)

## Residue

- Whether to extend mypy to `tests/` is deferred (task Open question); not an obligation here.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐ <one sentence deriving the verdict from the statuses>
