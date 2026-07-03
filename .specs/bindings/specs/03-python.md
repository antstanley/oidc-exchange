# Python Binding (`oidc-exchange`)

**Status:** Implemented · **Date:** 2026-07-02 · **Owner:** Ant Stanley · **Scope:** bindings/python

A PyO3 native extension wrapping [`crates/ffi`](01-ffi-core.md), built with maturin and
published to PyPI as `oidc-exchange`. The native module is `oidc_exchange._oidc_exchange`; a
thin Python package wraps it and adds ASGI/WSGI adapters.

## Responsibilities

- Expose the FFI `OidcExchange` as a Python class.
- Offer both a synchronous and an async request method.
- Provide ASGI and WSGI applications mountable in FastAPI/Starlette and Flask/Django.

## API (`python/oidc_exchange/__init__.py`)

```python
class OidcExchange:
    def __init__(self, *, config: str | None = None, config_string: str | None = None) -> None: ...
    def handle_request_sync(self, request: dict[str, Any]) -> dict[str, Any]: ...
    async def handle_request(self, request: dict[str, Any]) -> dict[str, Any]: ...
    def asgi_app(self) -> Any: ...
    def wsgi_app(self) -> Any: ...
    def shutdown(self) -> None: ...
```

A request `dict` is `{ "method", "path", "headers": dict[str,str], "body": bytes | str }`; the
response `dict` is `{ "status", "headers": dict[str,str], "body": bytes }`.

## Implementation

- **Rust (`src/lib.rs`)** — a `#[pyclass] OidcExchange` whose `#[new]` constructor calls FFI
  `from_file`/`new`, and `handle_request_sync` which extracts method/path/headers/body from
  the `PyDict`, releases the GIL (`py.allow_threads`) around the blocking FFI
  `handle_request` call, and re-acquires it to build the result dict. Other Python threads
  — including an asyncio event loop — keep running while a request is in flight.
  `shutdown` is a no-op.
- **Python (`python/oidc_exchange/`)** — `__init__.py` wraps the native class; `handle_request`
  (async) runs `handle_request_sync` in the default executor; `asgi_app`/`wsgi_app` delegate to
  the adapter factories. `_asgi.py` (`make_asgi_app`) collects the request body from `receive`,
  builds the request dict, awaits `handle_request`, and emits ASGI start+body messages.
  `_wsgi.py` (`make_wsgi_app`) reads `wsgi.input` and `HTTP_*` environ, calls
  `handle_request_sync`, and returns the status line, headers, and body. PEP 561 typing comes
  from `py.typed` plus the inline annotations on the (strictly pyright-checked) `__init__.py`;
  the native module's typed surface is the hand-curated `_oidc_exchange.pyi` stub.

## Distribution

maturin, `abi3` stable ABI targeting Python 3.10+ — `pyproject.toml` enables the
`pyo3/abi3-py310` feature so one wheel per platform works across 3.10–3.13. Linux wheels are
built in a `manylinux_2_28` container; platforms: `manylinux_2_28_{x86_64,aarch64}`,
`win_amd64`, `macosx_11_0_arm64`. An sdist is published alongside the wheels. See
[05-distribution.md](05-distribution.md).

## Tests

`tests/` (pytest + pytest-asyncio): construct with a local key + SQLite config; exercise the
health/keys/discovery endpoints, 404, and async handling. `httpx` (ASGI) and `werkzeug` (WSGI)
are dev dependencies for adapter-level tests.

## Assumptions and open questions

### Assumptions

- ASGI/WSGI hosts mount the adapter under a path prefix (e.g. `/auth`) and forward the
  remaining path to the binding.

### Decisions

- *Async wraps sync via executor.* **`handle_request` runs `handle_request_sync` in the default
  executor.** The FFI call is blocking; offloading keeps the event loop responsive without a
  second async runtime. (The older design's `pyo3_asyncio` future approach was not used.)
- *abi3 single wheel per platform.* **Built against the Python 3.10 stable ABI.** One wheel
  spans 3.10–3.13, cutting the build matrix.

### Open questions

- (None at this stage.)
