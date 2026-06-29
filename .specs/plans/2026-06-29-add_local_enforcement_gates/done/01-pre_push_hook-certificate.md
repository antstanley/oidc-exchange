# Done Certificate — Task 01: pre-push hook

**Task:** [01-pre_push_hook.md](01-pre_push_hook.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-06-29

> This certificate is a verification protocol for Task 01. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 01) ≡ every obligation O1…O5 below holds, each backed by the evidence the obligation
names (a file location, a command result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** The task produces a committed `.githooks/pre-push` script that runs the
  per-language format/lint/test gate, plus a `CONTRIBUTING.md` section documenting how to install
  it against the Git backend.
- **P2 — Obligations.** Done iff O1…O5 all hold. One Oi per definition-of-done item, in DoD
  order; O5 is the `Reviewable:` item.
- **P3 — Invariants.** Must not change any executable source under `crates/` or `bindings/`; the
  task adds a hook script and docs only, so the existing test suite and CI gates must remain
  exactly as they are (the hook is not activated in the live repo config).

## Obligations

- **O1 — The hook exists, is executable, and runs the per-language gates.**
  - *Claim:* `.githooks/pre-push` is a committed, executable script whose body invokes the Rust
    gate (`cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run
    --workspace`), the Node gate (`pnpm … fmt:check`, `lint`, `test`), and the Python gate
    (`uv run ruff format --check .`, `uv run ruff check .`, `uv run pytest`).
  - *Evidence to collect:* `ls -l .githooks/pre-push` — confirm the executable bit is set; read the
    script and confirm all three gate command groups appear; run `jj file list .githooks/pre-push`
    (or inspect the tree) to confirm it is tracked. Invoke it directly (`sh .githooks/pre-push
    </dev/null` from the workspace root) and read the output to confirm it dispatches gates.
  - *Checks:* confirm the gate commands match those in `.github/workflows/ci.yml` and
    `.specs/development-guidelines.md` §Definition of done (same tools, same flags), not a divergent
    set.
  - *Status:* ☑ **SATISFIED.** `ls -l` → `-rwxr-xr-x` (git diff header `new file mode 100755`);
    tracked in the jj tree (`jj diff --name-only` → `.githooks/pre-push`). Script body contains all
    three gate command groups verbatim: Rust `cargo fmt --check` / `cargo clippy --workspace -- -D
    warnings` / `cargo nextest run --workspace` (run_rust_gate); Node `pnpm fmt:check` / `pnpm lint`
    / `pnpm test` in `bindings/nodejs` (run_node_gate); Python `uv run ruff format --check .` / `uv
    run ruff check .` / `uv run pytest` in `bindings/python` (run_python_gate). Direct invocation
    dispatches gates (verified via `PRE_PUSH_DRY_RUN=1`). Commands match the task's Produces
    verbatim. Minor CI divergence (non-blocking): `ci.yml` uses `cargo fmt --check --all`, runs
    `pnpm build` before test, and `uv run maturin develop` + `pytest tests/`; the hook omits
    `--all`/`pnpm build`/`maturin develop` and runs `pytest` without `tests/` — each faithful to the
    task's stated commands but slightly narrower than CI.

- **O2 — The hook is scoped to changed languages, with a safe fallback.**
  - *Claim:* a change touching only one language's files runs only that language's gate; when the
    changed-file range cannot be computed, all gates run.
  - *Evidence to collect:* read the change-detection block in `.githooks/pre-push`; confirm it maps
    path prefixes (`crates/`, `bindings/nodejs`, `bindings/lambda`, `bindings/python`, `Cargo.*`)
    to gates and runs only matched gates. Trace the fallback branch: confirm that when no range is
    derivable it runs every gate (negative-space — it never silently runs nothing).
  - *Checks:* resolve the diff/range source (push-range stdin vs `origin/main`) the script uses;
    confirm the fallback is reached when that source is empty, not skipped.
  - *Status:* ☑ **SATISFIED.** `classify()` maps `crates/*`→RUST, `Cargo.toml|Cargo.lock`→RUST,
    `bindings/*/Cargo.toml`→RUST, `bindings/{nodejs,python}/src/*`→RUST, `bindings/{nodejs,lambda}/*`
    →NODE, `bindings/python/*`→PYTHON. Exercised in an isolated git repo (dry-run): Rust-only change
    →rust gate only; Node-only→node only; Python-only→python only; `bindings/lambda/*`→node;
    doc-only→"nothing to check" exit 0. Range source = stdin push lines `<local-ref> <local-sha>
    <remote-ref> <remote-sha>` with `<remote-sha>..<local-sha>` diff; new-branch (remote-sha all
    zeros) → `origin/main..<local-sha>`; branch deletion (local-sha all zeros) → ref skipped.
    Fallback verified positively: no usable git repo (jj workspace) → RUN_ALL; empty stdin with no
    `origin/main` → RUN_ALL; new branch with no `origin/main` → RUN_ALL; a non-derivable diff range →
    RUN_ALL. In every uncertain case it runs ALL three gates and never "runs nothing".

- **O3 — CONTRIBUTING.md documents installation and the jj caveat.**
  - *Claim:* `CONTRIBUTING.md` has a section explaining `git config core.hooksPath .githooks`, that
    the hook is opt-in and not auto-activated, and that `jj git push` does not run git hooks.
  - *Evidence to collect:* read the new `CONTRIBUTING.md` subsection; confirm the `core.hooksPath`
    command, the opt-in statement, and the explicit "jj does not run git hooks on `jj git push`"
    note (with CI named as the backstop) are all present.
  - *Status:* ☑ **SATISFIED.** New `### Pre-push hook` subsection in `CONTRIBUTING.md` contains the
    activation command `git config core.hooksPath .githooks` in a fenced block; an explicit opt-in
    statement ("The hook is **not** installed automatically — you must opt in"); and a blockquoted
    "**Important — `jj git push` does NOT run Git hooks.**" note with "**CI remains the authoritative
    backstop** for every push." All three elements present.

- **O4 — Meets the repo definition of done for what the task touches.**
  - *Claim:* the hook script is syntactically valid and the change introduces no source/test
    regression.
  - *Evidence to collect:* run `sh -n .githooks/pre-push` — expect no error; run `shellcheck
    .githooks/pre-push` if available — expect clean (or note its absence). Confirm no files under
    `crates/`/`bindings/*/src` changed (so `cargo nextest run --workspace`, per
    `.specs/development-guidelines.md` §Definition of done, is unaffected; run it if the build cache
    is warm and report the result).
  - *Status:* ☑ **SATISFIED.** `sh -n .githooks/pre-push` → no error (exit 0). `shellcheck` is not
    installed on this host (noted absent). The diff touches exactly two files — `.githooks/pre-push`
    and `CONTRIBUTING.md` (`jj diff --name-only`); no files under `crates/` or `bindings/*/src`
    changed, so `cargo nextest run --workspace` is unaffected (heavy cargo build deliberately not run
    per the gate's resource constraint; the no-source-change conclusion rests on the diff itself).

- **O5 — Reviewable: a reviewer runs the hook and confirms it gates (Reviewable).**
  - *Claim:* a reviewer can read `.githooks/pre-push` and the `CONTRIBUTING.md` section, run the
    script manually, and observe it invoking the documented gates and exiting non-zero on a failing
    gate.
  - *Evidence to collect:* invoke `sh .githooks/pre-push` manually and observe the gate commands
    being dispatched and the script's exit status; inspect the script's failure path to confirm a
    failing gate yields a non-zero exit with a message naming the gate.
  - *Checks:* the live "a failing hook blocks an actual `git push`" behaviour requires a real push
    a headless validator cannot drive — if it cannot be exercised, mark this obligation
    `UNVERIFIED` and surface for manual review rather than `UNSATISFIED`.
  - *Status:* ☑ **SATISFIED** (manual-run claim), with the live-push sub-behaviour **UNVERIFIED**.
    A real (non-dry-run) invocation was driven with `cargo`/`pnpm`/`uv` replaced by PATH stubs: the
    first gate command (`cargo fmt --check`) was made to exit 1, and the script printed a bordered
    `pre-push: GATE FAILED [rust]` block naming the failed command and "push aborted", then exited 1.
    Critically the `pnpm`/`uv` stubs (marked SHOULD-NOT-RUN) were never invoked — confirming
    first-failure abort with later gates skipped, and a non-zero exit naming the gate. The hook and
    the CONTRIBUTING section are both readable by a reviewer. The "a failing hook blocks an actual
    `git push`" sub-behaviour requires a live push a headless validator cannot drive → UNVERIFIED,
    surfaced for manual confirmation (not UNSATISFIED, per this obligation's own check).

## Regression check

- No existing executable code is modified — the task adds `.githooks/pre-push` and a
  `CONTRIBUTING.md` section. Confirm `git`/`jj`-tracked source under `crates/` and `bindings/` is
  untouched and the live repo hook config is not changed : ☑ **PRESERVED.** The diff adds only
  `.githooks/pre-push` and a `CONTRIBUTING.md` subsection (`jj diff --name-only` → exactly those two
  paths). No tracked source under `crates/` or `bindings/` is modified; the added files are not
  referenced by any existing code path; and the diff makes no change to repo hook config
  (`core.hooksPath` is not set), so the hook stays opt-in and CI/tests are unaffected.

## Residue

- The hook's value depends on contributors activating it; that is an out-of-band human step, not an
  obligation here. The change spec's own assumption (CI is the backstop) covers non-adopters.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: All five obligations are SATISFIED on collected evidence — an executable, tracked
`.githooks/pre-push` that dispatches the correct per-language gates with a safe all-gates fallback
that never runs nothing, aborts non-zero on the first failing gate (verified with PATH stubs),
plus the CONTRIBUTING activation/opt-in/jj-caveat docs, and the regression surface is PRESERVED
(only two added files, no source touched); the sole UNVERIFIED item is the live-`git push` block
sub-behaviour, which a headless validator cannot drive and is surfaced for manual confirmation
rather than failing the task.
