# Development Guidelines

**Status:** Implemented · **Date:** 2026-08-24 · **Owner:** Ant Stanley · **Scope:** Repo-wide

The rules of the road for everyone — humans and agents — writing code in `oidc-exchange`.
This page is canonical: a guideline here is a rule the repo adopts. It covers the toolchain,
the pervasive coding style, defensive coding, limits, version control, per-language
conventions, testing, repository hygiene, the emphases agents tend to slip on, and the
definition of done. It sits alongside [architecture-principles.md](architecture-principles.md)
in the global spec layer.

## Toolchain

| Tool | Version / channel | Notes |
|---|---|---|
| Rust | stable, edition 2021 | workspace at `crates/*`, `bindings/{nodejs,python}` |
| rustfmt | default channel | `cargo fmt --all`; `--check` in CI |
| clippy | latest | `cargo clippy --workspace -- -D warnings` (zero warnings) |
| cargo-nextest | latest | `cargo nextest run --workspace`; config in `.config/nextest.toml` |
| TypeScript | strict mode, `.ts` only | every package is ESM (`"type": "module"`) |
| pnpm | 11.9.0 | exact Corepack version (`corepack pnpm@11.9.0 install --frozen-lockfile --ignore-scripts`); frozen committed lockfiles are records of resolved inputs |
| oxfmt | latest | `pnpm format` / `pnpm format:check` |
| oxlint | latest | `pnpm lint` |
| tsc / astro check / svelte-check | latest | `pnpm typecheck` per TS workspace (`tsc --noEmit` for the bindings; `astro check` / `svelte-check` for the apps) |
| vitest | latest | `pnpm test` |
| Python | 3.10+ (abi3) | bindings/python |
| uv | latest | `uv sync`; never `pip` or manual virtualenvs |
| ruff | latest | `uv run ruff format .` and `uv run ruff check .` |
| pydantic | latest | data models requiring validation |
| pytest | latest | `uv run pytest` |
| pyright | latest, strict | `uv run pyright` over `bindings/python`; runs in CI |
| jujutsu (jj) | latest | sole VCS front end over a Git backend |
| lefthook | latest | pre-push gate (`lefthook.yml`); `pnpm install` installs it, `lefthook run pre-push` runs it by hand |
| CI | GitHub Actions | per-job permissions; lint, test, Node, Python, web-app, advisory, and signing-path gates |

## Tiger Style — the pervasive style

This project adopts **Tiger Style** as its pervasive coding style. This is not a
recommendation; it is the default. Deviations require a written reason in the change
description.

The short form: **be defensive and validate everything.** Assume any input you did not
produce is wrong. Assume any invariant you did not assert can be violated. Make every limit
explicit, every error handled, every assumption checked.

Design priorities — **safety, performance, developer experience, in that order.** When they
conflict, safety wins.

Load-bearing principles:

- **Zero technical debt.** Do it right the first time; ship a sound foundation.
- **Simple, explicit control flow.** No recursion (iterate with an explicit bound). No clever
  combinators that hide branches.
- **Limits on everything.** Every loop, retry, cache, and payload size has an explicit,
  declared upper bound.
- **Assertions are first-class code.** They detect programmer errors; the only correct
  response to a violated assertion is to crash. Aim for at least two assertions per function.
- **Always say *why*.** Comments and change descriptions explain the rationale, not the action.

## Defensive coding and assertions

### Where to validate

Validate at every boundary where data crosses from a place you do not control into one you do.

| Boundary | What to validate | How |
|---|---|---|
| HTTP request → handler | grant type, form fields, ids, sizes | parse/validate before the service sees it; reject unknown `grant_type` |
| Provider response → service | status, content type, claim shape | treat providers as adversarial; validate ID-token signature/iss/aud (see [05-provider-system](service/specs/05-provider-system.md)) |
| Adapter → core | domain invariants | assert preconditions at the top of each core function |
| Core → adapter | adapter contract | assert the shape of what the adapter returns |
| Store read | round-trip integrity | re-validate deserialized rows; do not trust stored JSON blindly |
| FFI boundary | method/path/header/body primitives | the binding validates before constructing the request |

### Assertions in Rust

- Use `assert!`, `debug_assert!`, `assert_eq!` liberally in core code; production keeps
  `assert!` enabled — no `--release`-only invariants.
- Average two or more assertions per function in cores: preconditions, postconditions,
  invariants. `assert!(true)` does not count.
- **Pair assertions** — enforce a property by at least two independent paths (assert at write
  time, re-validate at read).
- **Split compound assertions** (`assert!(a); assert!(b);`) so a failure points at the broken
  condition.
- **No `unwrap()`/`expect()` in production paths.** Tests and init-only code may use them; an
  init-time `expect()` carries a reason string.
- **No `panic!` for control flow.** Panics signal programmer error only.

### Assertions in TypeScript

- Use a small `invariant(condition, message)` helper that throws on a false condition; an
  `assertNever(x: never)` enforces exhaustive `switch`.
- Roughly two assertions per non-trivial function.
- Validate inbound data (Lambda events, request objects) before handing it to the binding;
  never cast network data with `as Foo`.

### Assertions in Python

- Use an `invariant(condition, message)` helper that **raises unconditionally** (`if not
  condition: raise ...`), not the bare `assert` statement — `assert` is stripped under `-O`.
  Reserve bare `assert` for test bodies.
- Roughly two invariant checks per non-trivial function.
- Use `pydantic` models to validate inbound structured data; use `typing.assert_never()` in
  the final branch of an exhaustive match.

### Errors are data, not exceptions

- Every error is a value with a typed reason. In Rust this is the per-crate `Error` enum
  (`thiserror`); the HTTP boundary translates it into a stable OAuth error code
  ([04-http-api](service/specs/04-http-api.md)). The FFI boundary collapses errors into
  `{code, message}`.
- **Every error is handled or explicitly propagated.** Swallowing an error is a bug — except
  the two documented best-effort paths (user-sync notifications, and token-verification
  failures in the RFC 7009 revoke response — backend and session-repo failures on `/revoke`
  propagate and map to 503), which log via `tracing` and are called out in the spec.
- **Retry policies are explicit and bounded** — the webhook adapter's `retries` + backoff is
  the model.
- **Never log a secret.** `WebhookConfig.secret` and `InternalApiConfig.shared_secret` have
  redacting `Debug`; keep new secret-bearing fields the same.

### Make invalid states unrepresentable

- Use the type system. Model state with enums (`UserStatus`, `AuditSeverity`,
  `AuditEventType`), not free strings; match exhaustively with no fallthrough.
- Prefer typed wrappers for structured strings over bare `String` where it prevents a class of
  bug (the `usr_`-prefixed id, the SHA-256 hash).

## Limits and bounds

Every limit is **declared as a named constant**, named with its units, and referenced
everywhere it applies. No magic numbers.

The *existence* of a limit is non-negotiable; concrete values live in the per-package specs,
not here. Limits this repo bounds: access/refresh token TTLs, JWKS cache TTL, webhook retry
count and timeout, user-list page size, LMDB max map size, HTTP request/body size at the
edge. Reaching a limit is an observable event — log it structured and reject or backpressure;
never silently drop.

## Version control

### Shared core

- **Commits are small and well-described.** One coherent change per commit.
- **Empty descriptions are not accepted.** Describe the *why* before pushing.
- **Conventional Commits** for the subject line: `type(scope): subject` (`feat`, `fix`,
  `docs`, `chore`, `refactor`, `test`, `build`, `ci`, `perf`, `style`).
- **`main` stays releasable.** Feature work happens on named bookmarks.
- **Do not rewrite published history** unless the change is yours and unmerged.
- **Destructive operations need explicit confirmation.**

### jujutsu

The repo is jj-managed (`.jj/` over a Git backend).

- **`jj` is the sole version-control front end.** Do not run `git commit`/`git add`/`git
  status` against the working copy — the index/working-copy mismatch is exactly what jj
  removes.
- **Describe before pushing.** `jj describe` sets the *why*.
- **Feature work happens on named bookmarks** (`jj bookmark create feat/x`); push with
  `jj git push`. Move `main` with `jj bookmark set main`.
- **Resolve conflicts in jj** (`jj resolve`), not by editing markers.
- **Destructive `jj` operations need explicit confirmation** — `jj abandon`, `jj op restore`,
  force-fetches, bookmark deletion.
- The `.jj/` directory is local and not committed.

## Rust conventions

### Formatting and linting

- `cargo fmt --all` clean before pushing (`cargo fmt --check --all` in CI).
- `cargo clippy --workspace -- -D warnings` clean — zero warnings.
- A committed `clippy.toml` configures `await-holding-invalid-types` with
  `tokio::sync::RwLockWriteGuard`, `tokio::sync::RwLockReadGuard`, and
  `tokio::sync::MutexGuard`, so `clippy::await_holding_invalid_type` fires at the binding
  site when an async-aware lock guard is alive across an `.await`. The better-known
  `clippy::await_holding_lock` covers only `std::sync` and `parking_lot` guards and does not
  catch tokio's, which is why the type list is configured deliberately. The stated rule
  behind the lint: **no lock guard may be alive across an `.await` that performs I/O**, and
  single-flight is expressed with its own primitive rather than obtained as a side effect of
  a data lock.

### Code style

- **Modules over files.** Many small files; a 1000-line `.rs` is a smell.
- **No business logic in `main.rs` or handlers.** Handlers parse, validate, call a core
  function, serialise the result. Business logic lives in `crates/core`.
- **Stay inside the hexagon.** No I/O in a core module — define a port, implement an adapter
  ([02-ports-and-adapters](service/specs/02-ports-and-adapters.md)).
- **Explicit fixed-width integers** (`u32`, `u64`) for domain values across serialisation.
- **Errors are `Result`** with one `Error` enum per crate; `From` impls translate vendor
  errors at the boundary — the core never sees a third-party error type.
- **No `unsafe`** outside an adapter that strictly needs it; any `unsafe` carries a `// SAFETY:`
  comment.
- **Hard limit: 70 lines per function** (a review gate) and **100 columns per line** (rustfmt).
- **No recursion;** iterate with an explicit bound.
- **Simpler return types win:** `()` > `bool` > integer > `Option<T>` > `Result<T, E>`.
- **Comments explain *why*,** in full sentences.

### Naming

- `snake_case` for functions/variables/modules/files; `CamelCase` for types/traits; acronyms
  in proper case (`HttpClient`, not `HTTPClient`).
- **Units last in identifiers**, descending significance: `latency_ms_max`.
- **No abbreviations** beyond ecosystem-standard short names (`ctx`, `cfg`, `id`).

### Testing

- `cargo nextest run --workspace`; the CI profile (`--profile ci`) adds retries and fail-fast.
- **Test pyramid.** In-module unit tests; core tests in `crates/core/tests/` using the
  `crates/test-utils` mocks (no network/filesystem); adapter integration tests in
  `crates/adapters/tests/` (HTTP adapters use `wiremock`, DynamoDB uses DynamoDB Local,
  external ones marked `#[ignore]`); server E2E in `crates/server/tests/` driving a full axum
  router with mock adapters.
- **Positive and negative space** — every accepted input has a paired rejected input.
- **Test the validity boundary** — one below a limit, at the limit, one above.
- **Determinism.** Use the deterministic mock key/id generators; no wall-clock or randomness
  in test bodies.

### Documentation

- Public items in library crates carry doc comments; each crate's `lib.rs` states what the
  crate is and the ports it offers.
- No bare `// TODO` without an owner and a tracking reference.

## TypeScript conventions

### Formatting and linting

- `pnpm format` (oxfmt), `pnpm lint` (oxlint `--deny-warnings`, so warnings fail the gate), and
  `pnpm typecheck` (tsc / `astro check` / `svelte-check`) clean before pushing. The only exemptions
  are per-file `.oxlintrc.json` overrides for two generated/scaffold files (the napi-generated
  `bindings/nodejs/index.js`; the SvelteKit `apps/admin-ui/src/lib/index.ts` placeholder).
- All source is `.ts`; all packages are ESM (`"type": "module"`); `require()` only via
  `createRequire` for loading native `.node` addons.

### Code style

- **No `any`.** Use `unknown` plus narrowing or a typed parser; casts are bugs unless
  justified in a comment.
- **Strict compiler settings are load-bearing.**
- **Domain types come from one source** (the binding's generated `.d.ts` / shared types),
  never hand-redefined.
- **Errors are values** across module boundaries; throw only on programmer error (the
  `invariant` path).
- **No silent fallthrough** — a `switch` over a discriminated union ends with `assertNever`.
- **70 lines/function** (review gate), **100 columns/line** (formatter).
- **Comments explain *why*.**

### Naming

- `camelCase` for functions/variables, `PascalCase` for types/classes,
  `SCREAMING_SNAKE_CASE` for module constants. Units last (`latencyMsMax`). No abbreviations.

### Testing

- vitest; tests in `__tests__/` or `*.test.ts`. Test pyramid, positive/negative space,
  validity boundaries, no flaky tests, injected clock/id.

### Documentation

- Public exports of shared packages carry doc comments; the entry module states the surface.
  No bare `// TODO` without an owner.

## Python conventions

### Formatting and linting

- `uv run ruff format .` and `uv run ruff check .` clean before pushing; warnings are errors.
- Managed exclusively with `uv` — never `pip` or manual virtualenvs.

### Code style

- **Type-annotate everything public;** no implicit `Any`.
- **`pydantic` models** for value objects and inbound validation; no trusting a raw `dict`.
- **Enums for state, not strings;** match exhaustively with `typing.assert_never`.
- **Errors are explicit** — raise typed exceptions; never a bare `except:`; catch the
  narrowest type.
- **No mutable default arguments.**
- **70 lines/function** (review gate), **100 columns/line**.
- **Comments explain *why*.**

### Naming

- `snake_case` for functions/variables/modules, `PascalCase` for classes,
  `SCREAMING_SNAKE_CASE` for constants. Units last (`latency_ms_max`). A leading underscore
  marks module-private names (`_asgi`, `_wsgi`, `_oidc_exchange`); respect it.

### Testing

- `uv run pytest`; tests in `tests/`. Use `httpx` (ASGI) and `werkzeug` (WSGI) for the adapter
  tests. Test pyramid, positive/negative space, validity boundaries, injected clock/id,
  Hypothesis for parsers/state machines.

### Documentation

- Public functions/classes/modules carry docstrings stating purpose and contract. No bare
  `# TODO` without an owner.

## Repository hygiene

- **`.specs/`** is the canonical home for specs and decisions; **`/docs`** holds the prose
  documentation the website renders.
- **Secrets never land in a committed TOML file** — reference them via `${VAR}` and supply via
  the environment.
- **Version parity** across `Cargo.toml`, `bindings/nodejs/package.json`, and
  `bindings/python/pyproject.toml` is enforced by the release pipeline's `validate` job
  ([bindings/specs/05-distribution](bindings/specs/05-distribution.md)).
- **CI is the enforcement gate** (`.github/workflows/ci.yml`): format-check, clippy, nextest,
  the napi build + vitest, the maturin build + pytest/Ruff/Pyright, web-app hygiene, the frozen
  Cargo/pnpm/Python advisory wrapper, and the source-derived signing-path policy run on every push
  and PR. See [distribution supply-chain gates](bindings/specs/05-distribution.md#supply-chain-gates).
- **Generated key material and local state are never committed.** `.gitignore` uses patterns
  `*.pem`, `*.p8`, `*.key`, `keys/`, `data/`, `lmdb/`, `*.db`, `*.sqlite`, and `*.sqlite3`, because
  documented setup flows write relative to their caller's directory. This is especially important
  under jj: the default `snapshot.auto-track = "all()"` records unignored new files on the next
  snapshot without a deliberate add step.
- **Dependency advisories are triaged, not carried silently.** Each committed graph is scanned.
  A carried finding requires the exact policy schema, reachability rationale, owner, review date,
  and expiry; unknown or expired findings and tool/DB/registry failures fail closed. Cargo
  unmaintained/yanked reports warn rather than masquerading as clean.
- **The pre-push hook** (managed by [lefthook](https://lefthook.dev) via `lefthook.yml`) runs format-check, lint, typecheck, and the fast test tier for every language the change touches; CI re-runs the same plus the slow/integration tier. A failing hook blocks the push; do not bypass it. `pnpm install` installs it (the root `prepare` script runs `lefthook install`); run it by hand any time with `lefthook run pre-push` (see [CONTRIBUTING.md](../CONTRIBUTING.md)). Note that `jj git push` does not run git hooks, so CI remains the backstop.
- **The 70-lines-per-function and two-assertions-per-function limits stay review gates, not hard lints.** A clippy `too_many_lines` lint at threshold 70 was evaluated and declined: existing functions exceed it (up to 134 lines across core, adapters, and the server crate), so enabling it under `-D warnings` would break the build without a sanctioned refactor. Assertion density is not lintable off the shelf. Both stay reviewer-enforced.
- **Generated native artifacts are not committed** — they are built per platform in CI.

## Guidelines for AI agents

These are not different rules — they are emphasis on where agents slip.

1. **The pervasive style applies to you too.** Defensive validation and explicit limits are
   not optional, even on a small change.
2. **Add assertions as you go.** Every function you touch leaves with at least two meaningful
   assertions.
3. **No silent error swallowing.** Every error is handled; every match on an enum is
   exhaustive. The only best-effort paths are the two documented ones (user-sync, and
   revoke's token-verification failures — never revoke's backend errors).
4. **Stay inside the architecture.** Adding I/O to a core module is the most common slip —
   define a port, implement an adapter, call into it.
5. **Do not add backwards-compat shims.** If a type changes, change every caller; there is no
   published API to preserve internally.
6. **Do not invent fields** not in the canonical types schema. Update
   `canonical-types.schema.json` and the prose first.
7. **Tests run before claiming complete.** "Compiles" is not "works"; run them and report the
   actual output.
8. **Test positive and negative space together.**
9. **Limits are explicit.** A new loop, retry, cache, or buffer ships with a named constant.
10. **Prefer small, frequent commits.**
11. **No comments that paraphrase the code.**
12. **Use `jj`, never `git`.** Do not run destructive VCS operations without confirmation.

## Definition of done

A change is done when:

- The behaviour is exercised by a test (unit, integration, or E2E as appropriate).
- The change includes **negative-space tests** for every new validation path.
- Every new or touched function has at least two meaningful assertions.
- Every new bound is a named constant in the relevant module.
- Format, lint, and the test suite pass locally for every language the change touches:
  - Rust: `cargo fmt`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace`.
  - TypeScript: `pnpm format:check`, `pnpm lint`, `pnpm typecheck`, `pnpm test`.
  - Python: `uv run ruff format --check .`, `uv run ruff check .`, `uv run pyright`, `uv run pytest`.
- If domain types changed, `canonical-types.schema.json` and the affected prose pages are
  updated together.
- The change description states the *why* and what changed at the architecture level.

## Assumptions and open questions

### Assumptions

- CI (`.github/workflows/ci.yml`) is the authoritative gate; a change that passes the five CI
  jobs (lint, test, nodejs-test, python-test, web-apps) satisfies the mechanical part of the
  definition of done.
- Contributors run `jj`, not `git`, against the working copy.

### Decisions

- *Tiger Style.* **The repo adopts Tiger Style (safety > performance > DX).** The Rust core is
  already `Result`/`thiserror`-based and IO is isolated behind ports, so an errors-as-data,
  assert-heavily discipline fits the existing grain.
- *Three toolchains, one CI.* **Rust (rustfmt/clippy/nextest), TS (oxfmt/oxlint/tsc/vitest+pnpm),
  Python (ruff/pyright/pytest+uv) run as CI jobs.** Each language uses its idiomatic tools; one
  workflow enforces them. The Astro/SvelteKit apps add a `web-apps` job (oxlint/oxfmt + `astro
  check`/`svelte-check`), and that job runs `pnpm test` for `apps/admin-ui`, whose session
  verification boundary is security-critical.
- *jj as the front end.* **Jujutsu is the sole VCS interface over the Git backend.** Avoids the
  index/working-copy mismatch of mixing `git` commands into a jj working copy.
- *Manual version parity, machine-checked.* **Versions are bumped by hand in three manifests
  and verified by the release `validate` job.** Keeps the bump explicit while preventing a
  mismatched publish.
- *Local enforcement gates.* **A [lefthook](https://lefthook.dev) pre-push hook (`lefthook.yml`, installed by `pnpm install`), strict pyright in CI, oxlint/oxfmt/tsc hygiene across the TS workspaces, and the size/assertion limits kept as documented review gates.** The hook runs the per-language format/lint/typecheck/fast-test gate before a push while CI remains the backstop (jj does not run git hooks on `jj git push`); pyright strict type-checks `bindings/python` in CI; the bindings and the Astro/SvelteKit apps gate on `pnpm lint`/`format:check`/`typecheck`; a clippy `too_many_lines` lint at 70 was declined because existing code exceeds it, so that limit and assertion density stay reviewer-enforced. This supersedes the earlier `core.hooksPath` shell-hook + mypy decision — see [changes/merged/2026-06-29-migrate_enforcement_to_lefthook_pyright.md](changes/merged/2026-06-29-migrate_enforcement_to_lefthook_pyright.md).

### Open questions

- A `clippy.toml` is committed, configuring `await-holding-invalid-types` only. Whether to
  extend it toward a pedantic-adjacent ruleset is still open; the file existing removes the
  obstacle but not the question.
