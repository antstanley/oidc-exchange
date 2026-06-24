# Bindings & distribution specs

The FFI core, the language bindings, and how every artifact ships (`crates/ffi`, `bindings/*`,
`install.sh`, `Dockerfile`, `.github/workflows`). These build on the global specs — read them
first:

- [../architecture-principles.md](../architecture-principles.md)
- [../development-guidelines.md](../development-guidelines.md)
- [../canonical-types.schema.json](../canonical-types.schema.json) — the shared `HttpRequest`/`HttpResponse` envelope

## Pages

| Page | Covers |
|---|---|
| [specs/00-overview.md](specs/00-overview.md) | how the Rust service is surfaced in other runtimes |
| [specs/01-ffi-core.md](specs/01-ffi-core.md) | `crates/ffi`: the wrapper every binding consumes |
| [specs/02-nodejs.md](specs/02-nodejs.md) | `@oidc-exchange/node` (napi-rs) |
| [specs/03-python.md](specs/03-python.md) | `oidc-exchange` (PyO3) + ASGI/WSGI adapters |
| [specs/04-lambda.md](specs/04-lambda.md) | `@oidc-exchange/lambda` event adapter |
| [specs/05-distribution.md](specs/05-distribution.md) | install script, Docker, release pipeline, version parity |
