# Done Certificate — Task 01: release GIL in handle_request_sync

**Task:** [01-release_gil_in_handle_request_sync.md](01-release_gil_in_handle_request_sync.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified   <!-- validator sets: Validated YYYY-MM-DD -->

> This certificate is a verification protocol for Task 01. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 01) ≡ every obligation O1…O6 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** `handle_request_sync` runs the blocking FFI `handle_request` with the GIL
  released via `py.allow_threads`, and a regression test proves a second Python thread makes
  progress while a request is in flight.
- **P2 — Obligations.** The task is done iff O1…O6 all hold. One Oi per definition-of-done item,
  in DoD order; O6 is the `Reviewable:` item.
- **P3 — Invariants.** Must not break the existing response-dict shape built at
  `bindings/python/src/lib.rs:90-103`, the FFI-error → `PyRuntimeError` mapping, or the async
  executor offload in `bindings/python/python/oidc_exchange/__init__.py:23-27` that the existing
  `test_async_health` exercises.

## Obligations

- **O1 — `handle_request_sync` wraps the FFI call in `py.allow_threads`, result dict built after.**
  - *Claim:* the FFI `handle_request` call runs inside `py.allow_threads(|| …)` and the
    `PyDict` for the response is constructed only after the closure returns (GIL re-held).
  - *Evidence to collect:* read `bindings/python/src/lib.rs` around lines 85-103; confirm the
    `self.inner.handle_request(&method, &path, headers, body)` call is the body of a
    `py.allow_threads` closure, the `FfiError` is mapped after the closure, and every
    `PyDict::new_bound(py)` / `set_item` call sits after the closure returns.
  - *Checks:* resolve `allow_threads` to the PyO3 `Python<'py>` method (the `py` bound at
    `lib.rs:48`), not a shadowing local; confirm `headers`/`body` are moved into the closure and
    `method`/`path` are borrowed, so the `Send` bound is met without cloning.
  - *Status:* ☐ unverified

- **O2 — Regression test proves progress under an in-flight call, red before / green after.**
  - *Claim:* a test in `bindings/python/tests/` shows a second Python thread advances while a
    `handle_request_sync` call is in flight; it fails against the pre-change `lib.rs` and passes
    after.
  - *Evidence to collect:* run the new test with `uv run pytest bindings/python -k <test name>` —
    expect PASS; then revert the `py.allow_threads` wrap locally (or `git stash`/checkout the
    prior `lib.rs`), rebuild the extension, and re-run — expect FAIL; restore the change. Read the
    test body and confirm it asserts the counter advanced during (not merely after) the call.
  - *Checks:* confirm the concurrency claim rests on the GIL release (the second thread runs
    Python that increments a counter), not on wall-clock sleeps that would pass regardless.
  - *Status:* ☐ unverified

- **O3 — Negative-space: error path still raises; async path still passes.**
  - *Claim:* an errored FFI request still maps to `PyRuntimeError` after the closure, and
    `test_async_health` still passes.
  - *Evidence to collect:* run `uv run pytest bindings/python/tests/test_handle_request.py` —
    expect all green including `test_async_health`; read the mapped-error branch after the
    `allow_threads` closure in `lib.rs` and confirm it raises `PyRuntimeError` from the
    `FfiError`. If a negative-path test exists, name it and expect PASS.
  - *Status:* ☐ unverified

- **O4 — Two meaningful assertions on the touched function; test timeout is a named constant.**
  - *Claim:* `handle_request_sync` carries a precondition on the extracted inputs and a
    postcondition on the built response, and the regression test's timeout is a named constant.
  - *Evidence to collect:* read `handle_request_sync` in `lib.rs`; confirm at least two distinct,
    split `assert!`/`debug_assert!` calls (one before the release on the inputs, one after on the
    response) that are not `assert!(true)`. Read the test and confirm the timeout/bound is a named
    constant (`SCREAMING_SNAKE_CASE` in Python), not a literal.
  - *Status:* ☐ unverified

- **O5 — Meets the repo definition of done.**
  - *Claim:* format, lint, type, and tests pass for the languages this task touches (Rust binding
    crate and Python).
  - *Evidence to collect:* run, from the repo root, `cargo fmt --all -- --check`,
    `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace`, and in
    `bindings/python`: `uv run ruff format --check .`, `uv run ruff check .`, `uv run pyright`,
    `uv run pytest` — expect all clean (commands from
    `.specs/development-guidelines.md` §Definition of done).
  - *Status:* ☐ unverified

- **O6 — Reviewable: test passes after, fails on revert; `uv run pytest` green (Reviewable).**
  - *Claim:* a reviewer runs the new regression test and sees it pass after the change and fail
    when `py.allow_threads` is reverted, and the `bindings/python` suite is green.
  - *Evidence to collect:* build the extension, run the named regression test (PASS), revert only
    the `allow_threads` wrap and re-run (FAIL), restore; run `uv run pytest` in `bindings/python`
    and observe all tests green.
  - *Status:* ☐ unverified

## Regression check

For each module the task touched, the validator traces one downstream caller:

- `bindings/python/python/oidc_exchange/__init__.py:23-27` `handle_request` offloads
  `handle_request_sync` to the executor; after the GIL release, `test_async_health` calls it →
  expect the async path still returns `status == 200` : ☐ (PRESERVED / REGRESSION)
- The response-dict consumers in `test_handle_request.py` (health/keys/discovery/404) read
  `response["status"]`/`["body"]` → expect the dict shape built after the closure is unchanged :
  ☐ (PRESERVED / REGRESSION)

## Residue

Notes for the validator: the async-counter vs thread-counter test shape is a task-local open
question (see plan.md); either is acceptable so long as O2's red-before/green-after evidence
holds. Multi-value header collapsing (existing `lib.rs:93-95` note) is out of scope and not an
obligation.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
