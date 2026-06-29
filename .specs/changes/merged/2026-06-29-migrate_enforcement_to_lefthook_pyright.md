# Change: Migrate enforcement gates to lefthook + pyright, add TS workspace hygiene

**Status:** Merged · **Date:** 2026-06-29 · **Merged:** 2026-06-29 · **Owner:** Ant Stanley · **Target:** Repo-wide (tooling)

Migrate the local enforcement gates landed by
[2026-06-24-add_local_enforcement_gates](2026-06-24-add_local_enforcement_gates.md) onto a
maintained hook manager and a faster type checker, and extend the TypeScript gate across every
pnpm workspace:

1. Replace the committed `.githooks/pre-push` shell script (wired via `core.hooksPath`) with a
   [**lefthook**](https://lefthook.dev) `pre-push` hook (`lefthook.yml`).
2. Replace **mypy** with **pyright** (strict) as the Python type checker.
3. Add **oxlint / oxfmt / TypeScript** hygiene with `lint`, `format`, `format:check`, and
   `typecheck` scripts to all four TS workspaces (`bindings/nodejs`, `bindings/lambda`,
   `apps/website`, `apps/admin-ui`), enforced by the lefthook pre-push hook and a CI `web-apps`
   job.

This closes the two `Open questions` the prior change spec left open (hook framework; mypy vs
pyright) and supersedes its matching `Decisions`.

---

## Motivation

The prior change wired the gates but deliberately closed two questions on the minimal-dependency
option: a raw `core.hooksPath` shell script and mypy. Both work, but:

- **Hook manager.** The shell hook re-implemented change-detection, all-gates fallback, and
  per-gate reporting by hand (~240 lines). lefthook does change-scoped, glob-filtered hook
  dispatch natively, is a single declarative `lefthook.yml`, installs itself from `pnpm install`,
  and offers `lefthook run pre-push` as a first-class manual entry point. Less bespoke shell to
  maintain for the same behaviour.
- **Type checker.** pyright is faster, has richer inference, and — unlike mypy — type-checks the
  real `__init__.py` implementation instead of skipping it because a sibling `__init__.pyi`
  shadows it. The migration surfaced (and fixed) a latent impl/stub type divergence mypy had
  masked.
- **TS coverage.** Hygiene scripts existed only on `bindings/nodejs` (and partially `lambda`),
  and there was no `typecheck` anywhere. The two apps had no lint/format gate at all.

A maintained manager + a stricter checker + uniform per-workspace scripts make the adopted
discipline cheaper to keep true.

---

## Affected spec pages

| Canonical page | Nature of change |
|---|---|
| [`.specs/development-guidelines.md`](../../development-guidelines.md) | Toolchain table (oxfmt `format`/`format:check`, new typecheck row, mypy→pyright, new lefthook row, CI jobs), Repository hygiene pre-push bullet, Definition of done (TS/Python lines), Decisions (Local enforcement gates) |
| [`.specs/bindings/specs/03-python.md`](../../bindings/specs/03-python.md) | API anchor + PEP 561 typing note: `__init__.pyi` removed; inline `__init__.py` types + native `_oidc_exchange.pyi` stub |

---

## Proposed changes

### Pre-push hook (Replace)

> Remove `.githooks/pre-push`. Add `lefthook.yml` with a `pre-push` hook running, glob-scoped to
> the language(s) in the push: the Rust gate (`cargo fmt --check` / `clippy -D warnings` /
> `nextest`); the Node and Lambda gates (`pnpm lint && format:check && typecheck && test`); the
> Python gate (`ruff format --check` / `ruff check` / `pyright` / `pytest`); the Website and
> Admin-UI gates (`pnpm lint && format:check && typecheck`). Add `lefthook` as a root devDependency
> and a root `prepare: lefthook install` script so `pnpm install` wires the git hook.

### Python type checker (Replace)

> In `bindings/python/pyproject.toml`, swap the `mypy` dev dependency and `[tool.mypy]` config for
> `pyright` and `[tool.pyright]` (`typeCheckingMode = "strict"`, `include`/`extraPaths = ["python"]`,
> `reportMissingModuleSource = false`). Add a hand-curated `_oidc_exchange.pyi` stub for the native
> module (replacing mypy's `ignore_missing_imports` override) and remove the now-redundant
> `__init__.pyi`. Swap the CI `Type-check` step to `uv run pyright`.

### TypeScript workspace hygiene (Add)

> Add `lint` (oxlint), `format`/`format:check` (oxfmt), and `typecheck` scripts to all four
> workspaces; standardize on `format`/`format:check` (rename from `fmt`/`fmt:check`). `typecheck`
> is `tsc --noEmit` for the bindings (new `bindings/nodejs/tsconfig.json`), `astro check` for the
> website, `svelte-check` for the admin UI. Add the missing devDependencies (`typescript`,
> `@types/node`, `@astrojs/check`, `oxlint`, `oxfmt`). Add a CI `web-apps` job.

---

## Type changes

None.

---

## Merge plan

1. Apply the `Proposed changes` blocks; bump the `development-guidelines.md` and `03-python.md`
   `**Date:**`.
2. No schema change.
3. Flip the two prior `Open questions` (hook framework, mypy-vs-pyright) to resolved and update the
   superseded `Decisions` on the guidelines page.
4. Update `.specs/README.md` Change-specs table.

---

## Assumptions and open questions

### Decisions

- *Hook manager.* **lefthook**, installed by the root `prepare` script. Declarative,
  glob-scoped, self-installing; `lefthook run pre-push` is the manual entry point. The jj caveat is
  unchanged — `jj git push` does not run git hooks, so CI stays the backstop.
- *Python type checker.* **pyright (strict)** over `uv run pyright`. Faster and stricter than
  mypy, and it checks the real `__init__.py` rather than being shadowed by a stub.
- *TS script naming.* **`format`/`format:check`** (not `fmt`/`fmt:check`) across all workspaces, so
  the trio is `lint` / `format` / `typecheck`.
- *App typecheck.* **Framework-native** — `astro check` for the website, `svelte-check` for the
  admin UI; oxlint/oxfmt cover the loose `.ts`/`.js` (they do not parse `.astro`/`.svelte`).
- *oxlint warnings are errors.* **`oxlint --deny-warnings`** on every workspace's `lint` script, so
  a warning fails the gate. The two pre-existing benign warnings are exempted *per file, per rule*
  via `.oxlintrc.json` overrides — `no-unused-vars` off for the napi-generated
  `bindings/nodejs/index.js`, `unicorn/no-empty-file` off for the SvelteKit
  `apps/admin-ui/src/lib/index.ts` scaffold placeholder — rather than disabling the rules globally
  or ignoring the files wholesale.

### Open questions

- None.
