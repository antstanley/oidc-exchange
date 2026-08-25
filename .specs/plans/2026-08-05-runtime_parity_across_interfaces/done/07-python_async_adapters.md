# Task 07 — Python async adapters

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted

**Implements:** [03-python.md §API](../../../bindings/specs/03-python.md), [03-python.md §Implementation](../../../bindings/specs/03-python.md), [03-python.md §Decisions](../../../bindings/specs/03-python.md), [00-overview.md §Shape](../../../bindings/specs/00-overview.md), source spec §Implementation notes 6
**Depends on:** 05
**Produces:** PyO3, ASGI, and WSGI paths that hand ordered/raw/bounded wire data to the FFI normaliser and expose a native coroutine
**Pointers:** `bindings/python/src/lib.rs:46-125`, `bindings/python/python/oidc_exchange/_asgi.py:20-60`, `bindings/python/python/oidc_exchange/_wsgi.py:20-73`, `bindings/python/tests/`

## Steps

- [x] Replace PyDict-only headers with ordered-pair extraction and return ordered response headers; validate direct mapping fields as typed errors.
- [x] Add `pyo3-async-runtimes` native coroutine support, or record the bounded executor fallback if abi3 support fails.
- [x] Forward ASGI `raw_path`, `query_string`, and header pairs without URI splicing; retain a declared non-raw fallback when the server lacks `raw_path`.
- [x] Forward WSGI `RAW_URI`/`REQUEST_URI` when available, preserve query separately, use ordered headers where representable, and declare fidelity qualifications otherwise.
- [x] Delete Python-side prefix stripping, decoded-path reconstruction, and header dict collapse; extend ASGI/WSGI/direct tests and corpus runners.

## Definition of done

- [x] Direct Python requests and adapter responses use ordered header sequences and preserve duplicate order where the host supplies it.
- [x] ASGI and WSGI never concatenate path and query or strip base paths; each reports `path_is_raw` truthfully.
- [x] ASGI/WSGI body handling remains capped through `limits()` and retains the 400/413 validation from task 03.
- [x] Native `handle_request` does not occupy the event loop; any abi3 fallback is tested and recorded as a qualification.
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: exercise ASGI and WSGI fixture cases showing raw-path qualification, duplicate headers, capped bodies, and async direct handling.

## Evidence

- `uv run pytest -q`: 29 passed. ASGI and WSGI boundary tests cover 400/413; direct async, typed mapping, and GIL-release cases pass.
- `uv run ruff format --check python tests`, `uv run ruff check python tests`, and `uv run pyright`: clean (Pyright 0 errors/warnings).
- `cargo clippy -p oidc-exchange-python --all-targets -- -D warnings`: passed.
- Qualification: the abi3-py310 distribution is retained, so `handle_request` uses the tested bounded `asyncio.to_thread` executor fallback rather than adding a CPython-version-specific pyo3 asyncio bridge. It does not occupy the event loop.
- Qualification: ASGI is raw only when the server supplies `scope['raw_path']`; fallback `path` is marked non-raw. Standard WSGI cannot represent duplicate headers and raw targets portably; `RAW_URI`/`REQUEST_URI` and the ordered `oidc_exchange.headers` server extension preserve them when supplied, otherwise `PATH_INFO` is marked non-raw.
