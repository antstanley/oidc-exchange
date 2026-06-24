# Bindings and Distribution — Overview

**Status:** Implemented · **Date:** 2026-06-24 · **Owner:** Ant Stanley · **Scope:** crates/ffi, bindings/*

How the Rust service is embedded in other runtimes and how every artifact is shipped.

> **Read first:** [.specs/architecture-principles.md](../../architecture-principles.md). The
> bindings re-use the service's axum router through `crates/ffi`; they do not reimplement any
> HTTP behaviour. The wire contract they expose is the same one described in
> [service/specs/04-http-api.md](../../service/specs/04-http-api.md).

## Shape

```
crates/ffi (OidcExchange: new/from_file + handle_request)
   │
   ├── bindings/nodejs   (napi-rs)   → @oidc-exchange/node     (class OidcExchange)
   │       └── bindings/lambda (TS)  → @oidc-exchange/lambda   (createHandler)
   └── bindings/python   (PyO3)      → oidc-exchange (PyPI)    (class OidcExchange + ASGI/WSGI)
```

Each binding wraps the FFI core's `OidcExchange` and translates an
[`HttpRequest`](../../canonical-types.schema.json) into the host's native request/response
shape. The request/response envelope is the repo-wide shared type.

## Detail pages

| Page | Covers |
|---|---|
| [01-ffi-core.md](01-ffi-core.md) | `crates/ffi`: the Rust wrapper every binding consumes |
| [02-nodejs.md](02-nodejs.md) | `@oidc-exchange/node` napi-rs binding |
| [03-python.md](03-python.md) | `oidc-exchange` PyO3 binding + ASGI/WSGI adapters |
| [04-lambda.md](04-lambda.md) | `@oidc-exchange/lambda` event adapter |
| [05-distribution.md](05-distribution.md) | binary install script, Docker, release pipeline, version parity |

## Assumptions and open questions

### Assumptions

- A binding process runs the FFI call from a non-tokio host thread (libuv, CPython), so the
  embedded runtime's `block_on` is safe.

### Decisions

- *One FFI core, many bindings.* **All bindings depend on `crates/ffi`.** A single Rust
  surface keeps behaviour identical across Node, Python, and Lambda.

### Open questions

- (None at this stage.)
