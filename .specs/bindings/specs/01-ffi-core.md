# FFI Core (`crates/ffi`)

**Status:** Implemented · **Date:** 2026-08-05 · **Owner:** Ant Stanley · **Scope:** crates/ffi

The shared Rust layer the language bindings consume. It wraps the server's axum router behind
a small synchronous interface and owns the tokio runtime so a non-async host can call it.

## Responsibilities

- Build an `AppService` and axum `Router` from a TOML config (re-using `crates/server`'s
  bootstrap), and own the tokio `Runtime` that drives them. Config supplied through `new`
  (inline string) or `from_file` passes the server's shared resolve —
  `OIDC_EXCHANGE__{section}__{key}` overrides, fail-closed `${VAR}` placeholder resolution,
  then the same load-time validation as the server's `load_config` (role, TTLs, allowlist,
  internal-API secret — [06-configuration.md](../../service/specs/06-configuration.md) →
  Loading order). An unresolvable placeholder or an invalid value is an `FfiError` at
  construction, never at request time; a literal `${…}` never reaches a running router.
- Convert a primitive HTTP request into an axum request, route it, and convert the response
  back to primitives.
- Map every error into a stable `FfiError`; never let a panic cross the FFI boundary.

## Public API (`crates/ffi/src/lib.rs`)

```rust
pub struct OidcExchange { /* runtime + router */ }

impl OidcExchange {
    pub fn new(config_toml: &str) -> Result<Self, FfiError>;   // from an inline TOML string
    pub fn from_file(path: &str) -> Result<Self, FfiError>;    // from a TOML file path

    pub fn handle_request(
        &self,
        method: &str,
        path: &str,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    ) -> Result<FfiResponse, FfiError>;
}

pub struct FfiResponse { pub status: u16, pub headers: Vec<(String, String)>, pub body: Vec<u8> }
pub struct FfiError    { pub code: String, pub message: String }   // impl Error + Display
```

## Request flow

`handle_request` builds an `http::Request` from the primitives, calls the cloned router on the
owned runtime via `runtime.block_on`, and collects the response status, headers, and body into
an `FfiResponse`. The router is built once in `new`/`from_file` and cloned per request (axum
routers are cheap to clone). Multiple `OidcExchange` instances with different configs can
coexist; there is no global state.

## Implementation layout

`crates/ffi/src/lib.rs` — the whole surface (`OidcExchange`, `FfiResponse`, `FfiError`).
Depends on `crates/server` (router construction), `crates/core`, and `tokio`/`axum`/`http`/
`tower`/`tracing`.

## Assumptions and open questions

### Assumptions

- The caller invokes `handle_request` from a non-tokio thread, so `block_on` does not nest
  runtimes.

### Decisions

- *Owned runtime, `block_on` per request.* **The struct owns its tokio runtime and blocks on
  each request.** FFI calls arrive synchronously from libuv/CPython threads; the binding layer
  is responsible for moving the call off the host's event-loop thread.
- *Errors as `{code, message}`.* **All Rust errors collapse to `FfiError`.** A flat, stable
  shape every language can surface without knowing the domain `Error` enum.
- *One resolve, differing sources.* **FFI config passes through the server's resolve; only the
  source set differs — the supplied document plus `OIDC_EXCHANGE__…` overrides, with no
  `OIDC_EXCHANGE_ENV` file overlay.** A second config pipeline is exactly how the published
  Node, Python, and Lambda packages came to load documented secret placeholders as literal text.

### Open questions

- `shutdown` is exposed by the bindings but is a no-op; whether the FFI core needs an explicit
  runtime-drain entry point is open.
