# Node.js Binding (`@oidc-exchange/node`)

**Status:** Implemented · **Date:** 2026-06-30 · **Owner:** Ant Stanley · **Scope:** bindings/nodejs

A napi-rs native module wrapping [`crates/ffi`](01-ffi-core.md). Published as
`@oidc-exchange/node` (ESM, `"type": "module"`).

## Responsibilities

- Expose the FFI `OidcExchange` as a JavaScript class.
- Marshal a JS request object to the FFI primitives and the `FfiResponse` back to a JS object.
- Load the correct platform-specific native binary at runtime.

## API (`index.d.ts`)

```typescript
interface HeaderEntry { name: string; value: string }
interface HttpRequest  { method: string; path: string; headers: HeaderEntry[]; body?: Buffer }
interface HttpResponse { status: number; headers: HeaderEntry[]; body: Buffer }
interface OidcExchangeOptions { config?: string; configString?: string }

class OidcExchange {
  constructor(options: OidcExchangeOptions);  // config = file path, configString = inline TOML
  handleRequest(request: HttpRequest): HttpResponse;   // synchronous
  shutdown(): void;                                    // no-op
}
```

`handleRequest` is **synchronous** — it calls the FFI core directly and returns the response.
The `HeaderEntry[]` shape maps onto the FFI `Vec<(String, String)>`. `body` is a `Buffer`.

## Implementation

`src/lib.rs` declares `#[napi(object)]` structs (`HeaderEntry`, `HttpRequest`, `HttpResponse`,
`OidcExchangeOptions`) and a `#[napi]` `OidcExchange` class whose constructor builds the FFI
instance from `config`/`configString` and whose `handle_request` forwards to
`ffi.handle_request`. `build.rs` runs `napi_build::setup()`. `index.js` is a platform-aware
loader that maps `{platform, arch}` to one of the optional platform packages (or a local
`oidc-exchange.node` fallback).

## Distribution

`napi build --release`, four targets: `x86_64-unknown-linux-gnu`,
`aarch64-unknown-linux-gnu`, `x86_64-pc-windows-msvc`, `aarch64-apple-darwin`. The native
binary for each is shipped as an optional dependency package under `npm/`:
`@oidc-exchange/{linux-x64-gnu,linux-arm64-gnu,win32-x64-msvc,darwin-arm64}`. See
[05-distribution.md](05-distribution.md).

The root `package.json` declares the four platform packages as `optionalDependencies` pinned to
the workspace version; `napi artifacts` copies each built `.node` into its `npm/<triple>` package
at release time. npm installs only the entry matching the host `{os, cpu}`; the `index.js` loader
resolves that package, falling back to a co-located `oidc-exchange.node`.

## Tests

`__tests__/index.test.ts` (vitest): construct with inline config (local Ed25519 key + SQLite),
exercise `/health`, `/keys`, `/.well-known/openid-configuration`, and 404 handling.

## Assumptions and open questions

### Assumptions

- The host generates an Ed25519 key and points config at a writable SQLite path for local use.

### Decisions

- *Synchronous `handleRequest`.* **The method blocks and returns the response directly.** The
  FFI core already owns a runtime; a sync surface is the simplest correct binding. (The older
  design's `Promise`-returning, `requestListener`-providing API was not built.)
- *Optional-dependency native packages.* **Per-platform `.node` binaries ship as
  optionalDependencies.** Standard napi-rs distribution; installers pull only their platform.

### Open questions

- A framework-agnostic `requestListener()` / `http.createServer` helper is not implemented;
  callers adapt `handleRequest` per framework today (see `examples/nodejs/*`).
