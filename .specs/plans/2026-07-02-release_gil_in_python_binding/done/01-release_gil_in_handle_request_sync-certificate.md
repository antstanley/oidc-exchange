# Done Certificate — Task 01: release GIL in handle_request_sync

**Task:** [01-release_gil_in_handle_request_sync.md](01-release_gil_in_handle_request_sync.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

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
  - *Status:* ☑ SATISFIED — `bindings/python/src/lib.rs:96-98`: `py.allow_threads(|| self.inner.handle_request(&method, &path, headers, body))` with `.map_err(... PyRuntimeError ...)` applied after the closure; `PyDict::new_bound(py)` / all `set_item` calls sit at lines 101-112, after the closure returns. Check: `py` resolves to the `Python<'py>` parameter bound at `lib.rs:48` (no intervening local named `py` in lines 48-96 — no shadowing); `headers: Vec<(String,String)>` and `body: Vec<u8>` are owned values moved into the closure, `&method`/`&path` borrowed; the FFI signature at `crates/ffi/src/lib.rs:84-90` takes owned data and returns plain Rust structs, and `cargo clippy -D warnings` compiles clean, confirming the `Send` bound is met with no cloning.

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
  - *Status:* ☑ SATISFIED — new test `test_handle_request_sync_releases_gil` in `bindings/python/tests/test_handle_request.py`. Green after: `uv run pytest tests/test_handle_request.py -k releases_gil` → PASSED. Red before: reverted only the `allow_threads` wrap to the pre-change `self.inner.handle_request(...)` form, rebuilt with `maturin develop`, re-ran → FAILED (`assert 15258.55… >= (0.05 * 10432810.0)` — counter thread at ~0.15% of its uncontended baseline rate); change restored and rebuilt, test PASSES again. Check: the assertion compares the counter thread's increments-per-second *during* the in-flight call (captured immediately after `handle_request_sync` returns) against a machine-calibrated baseline rate — the second thread runs a pure-Python increment loop, so the claim rests on the GIL being released, not on wall-clock sleeps.

- **O3 — Negative-space: error path still raises; async path still passes.**
  - *Claim:* an errored FFI request still maps to `PyRuntimeError` after the closure, and
    `test_async_health` still passes.
  - *Evidence to collect:* run `uv run pytest bindings/python/tests/test_handle_request.py` —
    expect all green including `test_async_health`; read the mapped-error branch after the
    `allow_threads` closure in `lib.rs` and confirm it raises `PyRuntimeError` from the
    `FfiError`. If a negative-path test exists, name it and expect PASS.
  - *Status:* ☑ SATISFIED — `uv run pytest tests/test_handle_request.py -v` → 9 passed, including `test_async_health PASSED` (named). `lib.rs:98`: `.map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?` sits immediately after the `allow_threads` closure and raises `PyRuntimeError` from the `FfiError`. Negative-path test exists: `test_handle_request_sync_invalid_method_raises_runtime_error` (invalid HTTP method → `RuntimeError`) → PASSED.

- **O4 — Two meaningful assertions on the touched function; test timeout is a named constant.**
  - *Claim:* `handle_request_sync` carries a precondition on the extracted inputs and a
    postcondition on the built response, and the regression test's timeout is a named constant.
  - *Evidence to collect:* read `handle_request_sync` in `lib.rs`; confirm at least two distinct,
    split `assert!`/`debug_assert!` calls (one before the release on the inputs, one after on the
    response) that are not `assert!(true)`. Read the test and confirm the timeout/bound is a named
    constant (`SCREAMING_SNAKE_CASE` in Python), not a literal.
  - *Status:* ☑ SATISFIED — preconditions at `lib.rs:88-89`: `assert!(!method.is_empty(), …)` and `assert!(!path.is_empty(), …)` (split, before the GIL release); postcondition at `lib.rs:116-119`: `debug_assert!(result.contains("status")?, …)` after the dict is built with the GIL re-held. None is `assert!(true)`. The test's timing bounds are all named module constants: `_GIL_TEST_WEBHOOK_DELAY_SECONDS`, `_GIL_TEST_WEBHOOK_CLIENT_TIMEOUT`, `_GIL_TEST_BASELINE_WINDOW_SECONDS`, `_GIL_TEST_MIN_RATE_FRACTION`, `_GIL_TEST_JOIN_TIMEOUT_SECONDS` — no bare literals in the assertions.

- **O5 — Meets the repo definition of done.**
  - *Claim:* format, lint, type, and tests pass for the languages this task touches (Rust binding
    crate and Python).
  - *Evidence to collect:* run, from the repo root, `cargo fmt --all -- --check`,
    `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace`, and in
    `bindings/python`: `uv run ruff format --check .`, `uv run ruff check .`, `uv run pyright`,
    `uv run pytest` — expect all clean (commands from
    `.specs/development-guidelines.md` §Definition of done).
  - *Status:* ☑ SATISFIED — all run 2026-07-02 from the repo root / `bindings/python`: `cargo fmt --all -- --check` clean; `cargo clippy --workspace -- -D warnings` clean; `cargo nextest run --workspace` → 215 passed, 10 skipped; `uv run ruff format --check .` → 7 files already formatted; `uv run ruff check .` → all checks passed; `uv run pyright` → 0 errors, 0 warnings; `uv run pytest` → 13 passed.

- **O6 — Reviewable: test passes after, fails on revert; `uv run pytest` green (Reviewable).**
  - *Claim:* a reviewer runs the new regression test and sees it pass after the change and fail
    when `py.allow_threads` is reverted, and the `bindings/python` suite is green.
  - *Evidence to collect:* build the extension, run the named regression test (PASS), revert only
    the `allow_threads` wrap and re-run (FAIL), restore; run `uv run pytest` in `bindings/python`
    and observe all tests green.
  - *Status:* ☑ SATISFIED — exercised end to end: built with `uv run maturin develop`; `test_handle_request_sync_releases_gil` PASSED; reverted only the `allow_threads` wrap, rebuilt, re-ran → FAILED (counter rate ~15.3k/s vs 10.4M/s baseline, below the 5% threshold); restored the wrap, rebuilt, then `uv run pytest` in `bindings/python` → 13 passed. Working copy verified restored to the implemented diff (`jj st`: only `src/lib.rs` and `tests/test_handle_request.py` modified).

## Regression check

For each module the task touched, the validator traces one downstream caller:

- `bindings/python/python/oidc_exchange/__init__.py:23-27` `handle_request` offloads
  `handle_request_sync` to the executor; after the GIL release, `test_async_health` calls it →
  expect the async path still returns `status == 200` : ☑ PRESERVED — `test_async_health PASSED`
  in the post-change run; the executor offload at `__init__.py:23-27` is untouched by the diff.
- The response-dict consumers in `test_handle_request.py` (health/keys/discovery/404) read
  `response["status"]`/`["body"]` → expect the dict shape built after the closure is unchanged :
  ☑ PRESERVED — the dict build at `lib.rs:101-112` is byte-identical to the pre-change shape
  (`status`/`headers`/`body`), and all pre-existing tests in the suite pass (13/13 green).

## Residue

Notes for the validator: the async-counter vs thread-counter test shape is a task-local open
question (see plan.md); either is acceptable so long as O2's red-before/green-after evidence
holds. Multi-value header collapsing (existing `lib.rs:93-95` note) is out of scope and not an
obligation.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O6 all SATISFIED with direct evidence — the `allow_threads` wrap and post-closure dict build read at `lib.rs:85-119`, the regression test proven red-on-revert (0.15% of baseline) / green-after (rebuilt both ways), the negative path and `test_async_health` green by name, both quality-gate suites (`cargo` and `uv`) fully clean, and both named downstream callers PRESERVED. (Residue note: the test resolved the open question with the thread-based counter loop against a slow local webhook endpoint, as permitted.)
