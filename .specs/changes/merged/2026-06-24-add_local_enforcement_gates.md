# Change: Add local enforcement gates for the development guidelines

**Status:** Merged · **Date:** 2026-06-24 · **Merged:** 2026-06-29 · **Owner:** Ant Stanley · **Target:** Repo-wide (tooling)

Add the enforcement mechanisms the development guidelines adopt as rules but do not yet wire up:
a pre-push hook that runs the format/lint/test gate locally, a Python strict type checker
(mypy or pyright) in CI, and lint gates for the function-size and assertion-density limits.

---

## Motivation

The [development guidelines](../../development-guidelines.md) adopt several disciplines whose
enforcement is currently CI-only or unenforced: there is no pre-push/pre-commit hook (failures
surface only after a push reaches CI), no Python static type checker (only `ruff` and `pytest`
run, so "type-annotate everything public" is unchecked), and the 70-lines-per-function and
two-assertions-per-function limits are review gates with no lint. The guidelines page records all
of these as Open questions.

A rule without a gate drifts. Wiring local and CI gates makes the adopted discipline mechanically
true, catches violations before they reach a reviewer, and shortens the feedback loop.

---

## Affected spec pages

| Canonical page | Nature of change |
|---|---|
| [`.specs/development-guidelines.md`](../../development-guidelines.md) | Move the wired gates from Open questions into the Toolchain table / Repository hygiene as facts; keep only genuinely undecided items as Open questions |

---

## Proposed changes

### `.specs/development-guidelines.md` → Repository hygiene (Modify)

> - **The pre-push hook** runs format-check, lint, and the fast test tier for every language the
>   change touches; CI re-runs the same plus the slow/integration tier. A failing hook blocks the
>   push; do not bypass it.

### `.specs/development-guidelines.md` → Toolchain (Add rows)

> | mypy (or pyright) | latest, strict | `uv run mypy` (or `pyright`) over `bindings/python`; runs in CI |

### `.specs/development-guidelines.md` → Open questions (Remove the resolved ones)

> Remove the pre-push-hook, Python-type-checker, and function-size/assertion-lint Open questions
> as each gate lands. Any limit lint that proves impractical stays documented as a review gate
> with a note.

---

## Type changes

None.

---

## Implementation notes

1. **Pre-push hook** — add a hook (a committed script under e.g. `.githooks/` wired via
   `core.hooksPath`, or a `lefthook`/`pre-commit` config) that runs, scoped to changed files
   where possible: `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`,
   `cargo nextest run --workspace`; `pnpm fmt:check && pnpm lint && pnpm test`;
   `uv run ruff format --check . && uv run ruff check . && uv run pytest`. Note: the repo uses jj;
   document how to install the hook against the Git backend.
2. **Python type checker** — add mypy (or pyright) in strict mode to `bindings/python`
   (`pyproject.toml` config), and a `python-typecheck` step to `.github/workflows/ci.yml`.
3. **Function-size lint** — enable clippy's `too_many_lines` (threshold 70 in `clippy.toml`) for
   Rust; for TS/Python use an oxlint/ruff rule or a CI check; treat as warnings-as-errors where
   the linter supports the threshold, otherwise keep as a documented review gate.
4. **Assertion density** is not lintable off the shelf; keep it a review gate and say so.
5. Document hook installation in `CONTRIBUTING.md`.

---

## Merge plan

1. As each gate lands, apply the matching `Proposed changes` block to the guidelines page and
   remove its Open question; bump the page `**Date:**`.
2. No schema change.
3. When all gates in this spec have shipped, flip `**Status:**` to `Merged`, stamp `**Merged:**`,
   move to `.specs/changes/merged/`.
4. Update `.specs/README.md`.

---

## Assumptions and open questions

### Assumptions

- Contributors will install the committed hook locally; CI remains the backstop for anyone who
  does not.

### Decisions

- *Gate the adopted rules.* **Wire pre-push, strict typing, and a size lint.** A rule the repo
  states but never enforces is the kind of drift the guidelines exist to prevent.

### Open questions

- Hook framework choice (raw `core.hooksPath` script vs `lefthook` vs `pre-commit`) is undecided;
  jj's Git backend interaction should drive it.
- mypy vs pyright for the Python checker is open.
- Whether an assertion-density gate can be automated at all, or stays a review-only gate, is
  unresolved.
