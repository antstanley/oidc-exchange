# FFI Core (`crates/ffi`)

**Status:** Implemented · **Date:** 2026-08-31 · **Owner:** Ant Stanley · **Scope:** crates/ffi

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
- Install the process-wide `tracing` subscriber at construction — the same `init_telemetry`
  the server binary runs at startup
  ([07-telemetry-and-audit.md](../../service/specs/07-telemetry-and-audit.md)) — after
  config resolution and before any adapter is built, so internal diagnostics (the 500 error
  mapping's log line, the panic-boundary record, adapter warnings) reach the embedder's
  stdout under `RUST_LOG` control. The install is idempotent and host-respecting: an
  already-set global dispatcher is retained untouched.

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
coexist; the one deliberate piece of process-global state is the `tracing` dispatcher — the
first construction (or the host) installs it, and every later instance observes it
unchanged, so the `[telemetry]` table of any instance after the first installer does not
re-route logs.

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
- *Constructor-installed telemetry, not a host-called hook.* **`OidcExchange` construction
  installs the subscriber itself, through the server's idempotent `init_telemetry`.** An
  exported `initTelemetry()` that Node, Lambda, and Python hosts must remember to call at
  cold start recreates the silent-discard bug for every host that forgets; installing at
  construction is fail-safe, and `try_init` keeps it correct when the host already owns a
  subscriber. Reusing the server's function keeps one telemetry pipeline for both
  entrypoints and adds no `tracing-subscriber` dependency to `crates/ffi` — the install
  rides the existing server-crate dependency.

### Open questions

- `shutdown` is exposed by the bindings but is a no-op; whether the FFI core needs an explicit
  runtime-drain entry point is open.


## Runtime parity update

- Build an `AppService` and axum `Router` from a TOML config (re-using `crates/server`'s
  bootstrap), and own the tokio `Runtime` that drives them. Config supplied through `new`
  (inline string) or `from_file` passes the same load-time validation as the server's
  `load_config` (role, TTLs, allowlist, internal-API secret —
  [06-configuration.md](../../service/specs/06-configuration.md) → Validation at load);
  invalid config is rejected as an `FfiError` at construction, never at request time.
- **Own request normalisation.** Turn the most faithful representation a host can supply —
  raw percent-encoded path bytes, the query string separately, an ordered list of header
  pairs, and a bounded body — into the `http::Request` the router routes. Percent-decoding,
  base-path stripping, header ordering, and body bounding all happen here and nowhere else.
- Publish the limits a host must respect before it buffers anything.
- Be total: no host input produces a panic across the boundary. Shaping failures become the
  same HTTP response the native server produces for the same wire bytes; only boundary
  failures with no HTTP meaning surface as `FfiError`.
```rust
pub struct OidcExchange { /* runtime + router + limits */ }

pub struct WireRequest {
    pub method: String,              // token as received; validated here
    pub raw_path: Vec<u8>,           // still percent-encoded, no query, no fragment
    pub query: Option<Vec<u8>>,      // still percent-encoded, no leading '?'
    pub headers: Vec<(String, String)>,  // ordered; duplicates preserved in wire order
    pub body: Vec<u8>,               // already bounded by the host against `limits()`
    pub hints: TransportHints,
}

pub struct TransportHints {
    /// False when the host could only supply an already-decoded path (a WSGI server
    /// exposing neither `RAW_URI` nor `REQUEST_URI`), so its parity claim is qualified.
    pub path_is_raw: bool,
}

pub struct NormalisationLimits { pub max_body_bytes: u64 }

impl OidcExchange {
    pub fn new(config_toml: &str) -> Result<Self, FfiError>;
    pub fn from_file(path: &str) -> Result<Self, FfiError>;

    pub fn limits(&self) -> NormalisationLimits;

    /// Total. Every `WireRequest` yields a response; `FfiError` is reserved for boundary
    /// failures with no HTTP meaning (a caught panic, a poisoned runtime).
    pub async fn handle(&self, request: WireRequest) -> Result<FfiResponse, FfiError>;
}

pub struct FfiResponse { pub status: u16, pub headers: Vec<(String, String)>, pub body: Vec<u8> }
pub struct FfiError    { pub code: String, pub message: String }   // impl Error + Display
```

`handle_request(method, path, headers, body)` remains as a deprecated synchronous shim that
splits `path` on the first `?`, marks `path_is_raw: false`, and forwards to `handle` on the
owned runtime. It is removed one major cycle after this change ships.
`handle` normalises in a fixed order, and every step is fallible rather than panicking:

1. **Method** — parsed with `http::Method::from_str`; an invalid token yields `400`.
2. **Path** — `raw_path` is validated as origin-form and left percent-encoded. An empty
   path normalises to `/` (PEP 3333 permits an empty `PATH_INFO` for a root-mounted
   application). A path that is not origin-form — no leading `/`, or an authority or scheme
   where a path belongs — yields `400`, never an absolute-form URI carrying a
   client-chosen authority.
3. **Query** — attached as-is; never concatenated by a host, so an encoded `?` or `#` in a
   path segment stays a literal octet instead of becoming URI structure.
4. **Base path** — stripped by `crates/server`'s `strip_prefix_at_segment_boundary`, the
   single implementation, at a path-segment boundary. `/authorize` is never mangled by a
   `/auth` prefix.
5. **Headers** — inserted in wire order with `HeaderMap::append`, so duplicates survive and
   first-wins consumers read what the native server reads. A header name or value the
   `http` crate rejects is dropped and counted, not fatal.
6. **Body** — rejected with `413` above `limits().max_body_bytes`; the router additionally
   carries `DefaultBodyLimit` set from the same value, so one number bounds all five shapes.

The router is cloned per request and awaited on the owned runtime. Every response body is
collected into `FfiResponse`.
Three guards, innermost outward. `CatchPanicLayer` stays nearest the handler so a caught
handler panic still passes back out through `request_id_layer` and carries its
`x-request-id`. A second `CatchPanicLayer` wraps the assembled router *and* the base-path
service, so a panic in the request-id, timeout, audit-context, or base-path layers is
contained too. `handle` then wraps its own body in `catch_unwind` and converts an escaped
unwind into `FfiError { code: "PANIC" }`, because the napi trampoline's behaviour under an
unwind is not established for this build (see Open questions). No `assert!`, `unwrap`, or
`expect` runs on host-supplied input anywhere on this path.
`conformance/corpus/` holds transport-agnostic request fixtures and the normalised request
each must produce. Four runners replay every fixture — the native hyper server, the FFI
directly, the Node binding (plus the Lambda adapter over synthesised events), and the
Python WSGI and ASGI adapters under a pinned server — and assert that method, decoded path,
query, ordered headers, and status agree across all five shapes. The corpus covers `%2F`
and `%2E%2E` in a path segment, an encoded `?` and `#`, duplicate `X-Forwarded-For` in both
orders, `/authorize` and `/auth/keys` as siblings of a `/auth` base path, an empty
`PATH_INFO`, a non-numeric `CONTENT_LENGTH`, a `CONTENT_LENGTH` of `sys.maxsize`, and a
body one byte over the cap. A fixture whose expectation a shape cannot meet records the
reason as a declared qualification against that shape's `TransportHints`, not as a skip.
The `conformance` CI job is a merge gate.
