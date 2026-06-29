# Task 01 — Pre-push hook

**Plan:** [plan.md](../plan.md) · **Certificate:** [01-pre_push_hook-certificate.md](01-pre_push_hook-certificate.md)

**Implements:** [.specs/changes/2026-06-24-add_local_enforcement_gates.md](../../../changes/merged/2026-06-24-add_local_enforcement_gates.md) §Implementation notes 1 & 5 (pre-push hook + `CONTRIBUTING.md` install docs); enables the [.specs/development-guidelines.md](../../../development-guidelines.md) §Repository hygiene pre-push-hook fact added in task 03.
**Depends on:** —
**Produces:** a committed `.githooks/pre-push` script that, scoped to the languages whose files changed, runs the Rust gate (`cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace`), the Node gate (`pnpm fmt:check && pnpm lint && pnpm test`), and the Python gate (`uv run ruff format --check . && uv run ruff check . && uv run pytest`); plus a `CONTRIBUTING.md` section documenting how to install it against the Git backend and noting jj does not run it on `jj git push`.
**Pointers:** new `.githooks/pre-push`; `CONTRIBUTING.md` (add a "Pre-push hook" subsection under Version Control or Code Standards); gate commands cribbed from `.github/workflows/ci.yml` and `.specs/development-guidelines.md` §Definition of done.

## Steps

- [x] Add an executable POSIX-`sh` script `.githooks/pre-push` that detects which top-level areas changed against the upstream ref (Rust under `crates/`/`bindings/*/src`/`Cargo.*`, Node under `bindings/nodejs`/`bindings/lambda`, Python under `bindings/python`) and runs only the affected language gate(s), falling back to running all gates when it cannot determine the range.
- [x] Have the Rust gate run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace`; the Node gate run `pnpm -C bindings/nodejs fmt:check && pnpm -C bindings/nodejs lint && pnpm -C bindings/nodejs test`; the Python gate run the three `uv run` commands inside `bindings/python`.
- [x] Make the script exit non-zero on the first failing gate (so a bad push is blocked) and print which gate failed; keep an explicit upper bound on work (each gate runs once) with no unbounded loop.
- [x] Mark the script executable and confirm it is committed with the executable bit.
- [x] Add a `CONTRIBUTING.md` subsection: how to activate the hook (`git config core.hooksPath .githooks`), that it is opt-in and not auto-activated, and an explicit note that `jj git push` does **not** run git hooks (the hook is a git-backend / manual-run convenience; CI remains the backstop).

## Definition of done

- [x] `.githooks/pre-push` exists, is executable, and a manual run on a clean tree executes the per-language gate commands (verify by invoking it directly, e.g. `sh .githooks/pre-push </dev/null`, and reading its output).
- [x] The script is scoped: a change touching only one language runs only that language's gate; when the changed-file set cannot be computed it runs all gates (negative-space: no-change / undetectable range still does something safe rather than nothing).
- [x] `CONTRIBUTING.md` documents installation via `core.hooksPath` and states that jj does not run git hooks on `jj git push`.
- [x] Meets the repo definition of done for the languages touched: the script itself passes `sh -n` (syntax) and shellcheck-clean where available; no other source code changes, so no test suite regression is introduced (confirm `cargo nextest run --workspace` still green if the build cache is warm).
- [x] Reviewable: a reviewer reads `.githooks/pre-push` and the new `CONTRIBUTING.md` section, runs the script manually, and confirms it invokes the documented gates and exits non-zero when a gate fails. (The live "blocks an actual `git push`" behaviour requires a real push and may be UNVERIFIED under a headless validator — surface for manual review, not a failure.)

## Open questions

- Whether the changed-file detection should diff against `@{upstream}`, `origin/main`, or the push range passed on stdin by git. The script picks a deterministic default (push-range stdin when present, else `origin/main`) and documents it inline.
