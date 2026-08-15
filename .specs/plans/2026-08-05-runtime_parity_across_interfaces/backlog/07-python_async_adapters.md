# Task 07 — Python async adapters

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted

**Implements:** [03-python.md §API](../../../bindings/specs/03-python.md), [03-python.md §Implementation](../../../bindings/specs/03-python.md), [03-python.md §Decisions](../../../bindings/specs/03-python.md), [00-overview.md §Shape](../../../bindings/specs/00-overview.md), source spec §Implementation notes 6
**Depends on:** 05
**Produces:** PyO3, ASGI, and WSGI paths that hand ordered/raw/bounded wire data to the FFI normaliser and expose a native coroutine
**Pointers:** `bindings/python/src/lib.rs:46-125`, `bindings/python/python/oidc_exchange/_asgi.py:20-60`, `bindings/python/python/oidc_exchange/_wsgi.py:20-73`, `bindings/python/tests/`

## Steps

- [ ] Replace PyDict-only headers with ordered-pair extraction and return ordered response headers; validate direct mapping fields as typed errors.
- [ ] Add `pyo3-async-runtimes` native coroutine support, or record the bounded executor fallback if abi3 support fails.
- [ ] Forward ASGI `raw_path`, `query_string`, and header pairs without URI splicing; retain a declared non-raw fallback when the server lacks `raw_path`.
- [ ] Forward WSGI `RAW_URI`/`REQUEST_URI` when available, preserve query separately, use ordered headers where representable, and declare fidelity qualifications otherwise.
- [ ] Delete Python-side prefix stripping, decoded-path reconstruction, and header dict collapse; extend ASGI/WSGI/direct tests and corpus runners.

## Definition of done

- [ ] Direct Python requests and adapter responses use ordered header sequences and preserve duplicate order where the host supplies it.
- [ ] ASGI and WSGI never concatenate path and query or strip base paths; each reports `path_is_raw` truthfully.
- [ ] ASGI/WSGI body handling remains capped through `limits()` and retains the 400/413 validation from task 03.
- [ ] Native `handle_request` does not occupy the event loop; any abi3 fallback is tested and recorded as a qualification.
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: exercise ASGI and WSGI fixture cases showing raw-path qualification, duplicate headers, capped bodies, and async direct handling.
