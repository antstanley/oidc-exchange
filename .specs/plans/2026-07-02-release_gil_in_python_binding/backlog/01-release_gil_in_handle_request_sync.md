# Task 01 — release GIL in handle_request_sync

**Plan:** [plan.md](../plan.md) · **Certificate:** [01-release_gil_in_handle_request_sync-certificate.md](01-release_gil_in_handle_request_sync-certificate.md)

**Implements:** [.specs/bindings/specs/03-python.md](../../../bindings/specs/03-python.md) §Implementation (the "Rust (`src/lib.rs`)" bullet — the GIL-release behaviour) and §Decisions ("Async wraps sync via executor" / "keeps the event loop responsive")
**Depends on:** —
**Produces:** `handle_request_sync` runs the blocking FFI `handle_request` with the GIL released via `py.allow_threads`, and a regression test demonstrates a second Python thread makes progress while a request is in flight
**Pointers:** `bindings/python/src/lib.rs:85-88` (the FFI call to wrap) and `:90-103` (the result-dict build that must stay GIL-held); `crates/ffi/src/lib.rs:84-116` (the `Send`, owned-data FFI signature); `bindings/python/tests/test_handle_request.py` (existing suite to extend); `bindings/python/python/oidc_exchange/__init__.py:23-27` (the executor offload the release unblocks)

## Steps

- [ ] In `handle_request_sync`, after the inputs (`method`, `path`, `headers`, `body`) are extracted (by `lib.rs:83`), wrap only the FFI call in the GIL-releasing closure: `py.allow_threads(|| self.inner.handle_request(&method, &path, headers, body))`, then map the `FfiError` to `PyRuntimeError` and build the `PyDict` after the closure returns with the GIL re-held.
- [ ] Confirm the closure satisfies `allow_threads`' `Send` bound with no restructuring (the FFI takes owned `Vec<(String,String)>`/`Vec<u8>` and returns plain Rust structs); keep `headers`/`body` moved into the closure and `&method`/`&path` borrowed across it.
- [ ] Add at least two meaningful assertions to `handle_request_sync`: a precondition on the extracted inputs before the release (e.g. `method`/`path` are non-empty) and a postcondition on the built response after re-acquiring the GIL (e.g. the result dict carries `status`), each split so a failure points at one condition.
- [ ] Add a regression test in `bindings/python/tests/` that proves the GIL is dropped: run `handle_request_sync` on one thread while a second Python thread advances a counter (or a pytest-asyncio counter task against a slow/mock-delayed endpoint), and assert the counter advanced during the in-flight call. Keep the test deterministic with an explicit timeout bound as a named constant.
- [ ] Confirm the existing suite still passes, including the async offload path (`test_async_health`) and the error path (an errored request still raises `PyRuntimeError`).

## Definition of done

- [ ] `handle_request_sync` wraps the FFI `handle_request` in `py.allow_threads` and builds the response `PyDict` only after the closure returns (GIL re-held).
- [ ] A regression test shows a second Python thread makes progress while a `handle_request_sync` call is in flight, and it fails against the pre-change `lib.rs` yet passes after the change.
- [ ] Negative-space holds: an errored FFI request still maps to `PyRuntimeError` after the closure, and the existing async/executor path (`test_async_health`) still passes.
- [ ] `handle_request_sync` carries at least two meaningful assertions (a precondition on the extracted inputs, a postcondition on the built response), and the test's timeout bound is a named constant.
- [ ] Meets the repo definition of done (Rust `cargo fmt` / `cargo clippy --workspace -- -D warnings` / `cargo nextest run --workspace`; Python `uv run ruff format --check .` / `uv run ruff check .` / `uv run pyright` / `uv run pytest` for the binding — see plan.md baseline).
- [ ] Reviewable: a reviewer runs the new regression test and sees it pass after the change and fail when `py.allow_threads` is reverted, and runs `uv run pytest` in `bindings/python` green.

## Open questions

- Whether a deterministic slow endpoint exists for the async counter variant, or the thread-based counter loop is the reliable shape — resolved while writing the test (see plan.md Open questions).
