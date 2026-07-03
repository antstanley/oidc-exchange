# Change: Release the GIL around the blocking FFI call in the Python binding

**Status:** Merged · **Date:** 2026-07-01 · **Merged:** 2026-07-03 · **Owner:** Ant Stanley · **Target:** bindings/python

Wrap the blocking FFI call in `handle_request_sync` in `py.allow_threads` so the GIL is
released while the Tokio runtime services the request. Today the executor thread that
`handle_request` offloads to holds the GIL for the whole call, freezing the asyncio event
loop — the exact stall the offload exists to prevent.

---

## Motivation

[03-python.md](../bindings/specs/03-python.md) decides that "offloading keeps the event loop
responsive": the async `handle_request` runs `handle_request_sync` in the default executor.
But `handle_request_sync` (`bindings/python/src/lib.rs:46-104`) never calls
`py.allow_threads` — the FFI `handle_request` (and the `runtime.block_on` inside it) runs
with the GIL held. The executor thread therefore blocks every other Python thread,
including the event-loop thread, for the duration of the call: any slow upstream provider
or webhook delivery freezes the entire ASGI application, not just the one request. The spec
describes the intended end state; the code does not deliver it.

The fix is the standard PyO3 pattern: extract everything needed from the Python objects
first, release the GIL for the blocking section, and re-acquire it to build the response
dict.

---

## Affected spec pages

| Canonical page | Nature of change |
|---|---|
| [`.specs/bindings/specs/03-python.md`](../bindings/specs/03-python.md) | Implementation bullet notes the GIL release; the "keeps the event loop responsive" decision already describes the end state — spec ahead of code |

---

## Proposed changes

### `.specs/bindings/specs/03-python.md` → Implementation (Modify)

> - **Rust (`src/lib.rs`)** — a `#[pyclass] OidcExchange` whose `#[new]` constructor calls FFI
>   `from_file`/`new`, and `handle_request_sync` which extracts method/path/headers/body from
>   the `PyDict`, releases the GIL (`py.allow_threads`) around the blocking FFI
>   `handle_request` call, and re-acquires it to build the result dict. Other Python threads
>   — including an asyncio event loop — keep running while a request is in flight.
>   `shutdown` is a no-op.

---

## Type changes

None.

---

## Implementation notes

1. `bindings/python/src/lib.rs:85-88` — all inputs (`method`, `path`, `headers`, `body`) are
   already extracted into owned Rust values by line 83; wrap the FFI call:
   `py.allow_threads(|| self.inner.handle_request(&method, &path, headers, body))`, then map
   the error and build the `PyDict` after the closure returns (GIL re-held).
2. `oidc_exchange_ffi::OidcExchange::handle_request` takes only owned/`Send` data and the
   response is plain Rust structs, so the closure satisfies `allow_threads`' `Send` bound
   with no restructuring.
3. Test: pytest-asyncio — issue a request against a deliberately slow endpoint (or a mock
   provider with a delay) on the executor while the event loop concurrently ticks a counter
   task; assert the counter advances during the call. A simpler thread-based variant
   (`threading.Thread` incrementing under a timeout) works without an async fixture.

---

## Merge plan

1. Apply the `Proposed changes` block to `03-python.md`; bump its `**Date:**`.
2. No schema change.
3. Flip `**Status:**` to `Merged`, stamp `**Merged:**`, move to `.specs/changes/merged/`.
4. Update `.specs/README.md`.

---

## Assumptions and open questions

### Assumptions

- No Python callbacks are invoked from inside the FFI call (none exist today), so releasing
  the GIL across it is sound.

### Decisions

- *Release, don't rearchitect.* **Keep the sync-in-executor design; just drop the GIL during
  the blocking section.** A native-async (`pyo3-async-runtimes`) surface remains a possible
  future change; this one-line-shaped fix delivers the documented behaviour now.

### Open questions

- (None at this stage.)
