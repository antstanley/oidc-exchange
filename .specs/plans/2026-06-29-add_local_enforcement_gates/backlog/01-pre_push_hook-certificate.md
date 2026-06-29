# Done Certificate — Task 01: pre-push hook

**Task:** [01-pre_push_hook.md](01-pre_push_hook.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-06-29 — unverified   <!-- validator sets: Validated YYYY-MM-DD -->

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
  - *Status:* ☐ unverified

- **O2 — The hook is scoped to changed languages, with a safe fallback.**
  - *Claim:* a change touching only one language's files runs only that language's gate; when the
    changed-file range cannot be computed, all gates run.
  - *Evidence to collect:* read the change-detection block in `.githooks/pre-push`; confirm it maps
    path prefixes (`crates/`, `bindings/nodejs`, `bindings/lambda`, `bindings/python`, `Cargo.*`)
    to gates and runs only matched gates. Trace the fallback branch: confirm that when no range is
    derivable it runs every gate (negative-space — it never silently runs nothing).
  - *Checks:* resolve the diff/range source (push-range stdin vs `origin/main`) the script uses;
    confirm the fallback is reached when that source is empty, not skipped.
  - *Status:* ☐ unverified

- **O3 — CONTRIBUTING.md documents installation and the jj caveat.**
  - *Claim:* `CONTRIBUTING.md` has a section explaining `git config core.hooksPath .githooks`, that
    the hook is opt-in and not auto-activated, and that `jj git push` does not run git hooks.
  - *Evidence to collect:* read the new `CONTRIBUTING.md` subsection; confirm the `core.hooksPath`
    command, the opt-in statement, and the explicit "jj does not run git hooks on `jj git push`"
    note (with CI named as the backstop) are all present.
  - *Status:* ☐ unverified

- **O4 — Meets the repo definition of done for what the task touches.**
  - *Claim:* the hook script is syntactically valid and the change introduces no source/test
    regression.
  - *Evidence to collect:* run `sh -n .githooks/pre-push` — expect no error; run `shellcheck
    .githooks/pre-push` if available — expect clean (or note its absence). Confirm no files under
    `crates/`/`bindings/*/src` changed (so `cargo nextest run --workspace`, per
    `.specs/development-guidelines.md` §Definition of done, is unaffected; run it if the build cache
    is warm and report the result).
  - *Status:* ☐ unverified

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
  - *Status:* ☐ unverified

## Regression check

- No existing executable code is modified — the task adds `.githooks/pre-push` and a
  `CONTRIBUTING.md` section. Confirm `git`/`jj`-tracked source under `crates/` and `bindings/` is
  untouched and the live repo hook config is not changed : ☐ (PRESERVED / REGRESSION)

## Residue

- The hook's value depends on contributors activating it; that is an out-of-band human step, not an
  obligation here. The change spec's own assumption (CI is the backstop) covers non-adopters.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐ <one sentence deriving the verdict from the statuses>
