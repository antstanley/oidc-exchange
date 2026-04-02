# Distribution & Language Bindings Design

**Date:** 2026-04-02
**Status:** Draft
**Scope:** Binary distribution, Docker images, install script, Node.js FFI bindings, Python FFI bindings, documentation updates

## Overview

Improve the getting-started experience for oidc-exchange by shipping prebuilt binaries, Docker images, a one-line install script, and FFI bindings for Node.js and Python. All artifacts are published from a single monorepo with a unified release triggered by a semver tag on the `main` bookmark.

## Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Integration model | FFI bindings (in-process) | Tightest integration, no separate server process |
| Repo structure | Monorepo | Atomic releases, single CI, shared Rust compilation |
| Node.js FFI | napi-rs | Mature, excellent cross-platform support, generates TypeScript types |
| Python FFI | PyO3 + maturin | Standard for Rust-Python FFI, builds manylinux wheels |
| Versioning | Semantic versioning (semver) | Industry standard, consistent across all artifacts |
| Release trigger | Git tag on main bookmark | `v1.0.0` tag triggers full release pipeline |
| Docker registries | ghcr.io + Docker Hub | Maximum reach |
| Min Node.js | 22 | Aligns with current LTS, simplifies napi target |
| Min Python | 3.10 | Required by Django 5.x |
| Build approach | GitHub Actions matrix with native runners | Native builds for most targets, cross only for arm64 Linux |

## Supported Platforms

| Target | OS | Arch | Binary name | Docker | Node.js | Python |
|--------|----|------|-------------|--------|---------|--------|
| `x86_64-unknown-linux-gnu` | Linux | x86_64 | `oidc-exchange-linux-x64` | Yes | Yes | Yes (`manylinux_2_28_x86_64`) |
| `aarch64-unknown-linux-gnu` | Linux | arm64 | `oidc-exchange-linux-arm64` | Yes | Yes | Yes (`manylinux_2_28_aarch64`) |
| `x86_64-pc-windows-msvc` | Windows | x86_64 | `oidc-exchange-windows-x64.exe` | No | Yes | Yes (`win_amd64`) |
| `aarch64-apple-darwin` | macOS | arm64 | `oidc-exchange-darwin-arm64` | No | Yes | Yes (`macosx_11_0_arm64`) |

## Sub-project Decomposition

### Build order

```
Sub-project 1 (FFI crate) ──┬──> Sub-project 3 (Node.js bindings) ──┐
                             │                                        ├──> Sub-project 5 (Docs)
                             └──> Sub-project 4 (Python bindings) ───┘
                                                                      │
Sub-project 2 (Binary dist) ─────────────────────────────────────────┘
```

Sub-projects 1 and 2 are independent. Sub-projects 3 and 4 depend on 1 but are independent of each other. Sub-project 5 depends on all others.

---

## Sub-project 1: FFI Crate (`crates/ffi`)

### Purpose

Shared Rust layer that both napi-rs and PyO3 consume. Wraps the existing `AppService` + Axum router into a C-compatible interface.

### Location

`crates/ffi/` — added as a workspace member in root `Cargo.toml`.

### API

```rust
pub struct OidcExchange {
    runtime: tokio::runtime::Runtime,
    router: axum::Router,
}

impl OidcExchange {
    /// Initialize from a TOML config string.
    /// Bootstraps AppService with all adapters, builds the Axum router,
    /// and spawns a Tokio runtime.
    pub fn new(config: &str) -> Result<Self, FfiError>;

    /// Initialize from a TOML config file path.
    pub fn from_file(path: &str) -> Result<Self, FfiError>;

    /// Handle an HTTP request.
    /// Translates primitive types into an Axum request, routes it,
    /// and returns primitive response types.
    pub fn handle_request(
        &self,
        method: &str,
        path: &str,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    ) -> Result<FfiResponse, FfiError>;
}

pub struct FfiResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

pub struct FfiError {
    pub code: String,
    pub message: String,
}
```

### Key design decisions

- **Tokio runtime management:** The `OidcExchange` struct owns its Tokio runtime. `handle_request` uses `runtime.block_on()` to execute the async Axum handler synchronously. This is safe because the FFI call originates from a non-Tokio thread (Node.js libuv or Python's thread pool).
- **Router reuse:** The Axum router is built once during `new()` and reused for every request via `Router::clone()` (Axum routers are cheap to clone).
- **No global state:** Multiple `OidcExchange` instances can coexist with different configs.
- **Error mapping:** All Rust errors are converted to `FfiError` with a code and message. No panics cross the FFI boundary.

### Dependencies

- `crates/core` (service, domain types)
- `crates/adapters` (all adapter implementations)
- `crates/providers` (Apple provider)
- `crates/server` (router construction, middleware) — the router-building logic will need to be extracted into a reusable function that both `main.rs` and `crates/ffi` can call.

### Tests

- Unit tests: initialize with noop/local adapters, send health/JWKS/discovery requests, verify responses.
- Integration tests: full token exchange flow using mock provider (wiremock) and local key manager.

---

## Sub-project 2: Binary Distribution & Docker

### GitHub Actions Workflows

#### `.github/workflows/ci.yml` — Continuous Integration

Triggered on push to any bookmark and PRs.

**Jobs:**
1. **lint** — `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`
2. **test** — `cargo nextest run --workspace` (default profile)
3. **nodejs-test** — Build Node.js bindings, run jest tests
4. **python-test** — Build Python bindings, run pytest tests

#### `.github/workflows/release.yml` — Release Pipeline

Triggered when a tag matching `v*.*.*` is pushed.

**Jobs:**

1. **validate**
   - Extract version from tag
   - Verify `Cargo.toml` workspace version matches
   - Verify `bindings/nodejs/package.json` version matches
   - Verify `bindings/python/pyproject.toml` version matches

2. **build-binaries** (matrix: 4 targets)
   - Build release binary for each target
   - Generate SHA256 checksum file
   - Run smoke test (start binary, hit `/health`)
   - Upload artifacts

3. **build-docker**
   - Multi-arch build via `docker buildx` (`linux/amd64`, `linux/arm64`)
   - Push to ghcr.io and Docker Hub
   - Tags: `latest`, `v1.0.0`, `v1.0`, `v1`
   - Smoke test: run container, hit `/health`

4. **build-nodejs** (matrix: 4 platforms)
   - napi-rs build for each platform
   - Publish platform packages to npm
   - Publish main `@oidc-exchange/node` package
   - Post-publish smoke test in clean environment

5. **build-python** (matrix: 4 platforms)
   - maturin build for each platform
   - Publish wheels to PyPI
   - Post-publish smoke test in clean environment

6. **create-release** (depends on all above)
   - Create GitHub Release
   - Upload binaries and checksums
   - Generate changelog from commits since last tag

### Install Script: `install.sh`

Located at repo root.

**Behavior:**
- Detects OS via `uname -s` → `Linux`, `Darwin`
- Detects arch via `uname -m` → `x86_64`, `aarch64`/`arm64`
- Maps to binary name (e.g., `Linux` + `x86_64` → `oidc-exchange-linux-x64`)
- Rejects unsupported combinations (e.g., x86 macOS) with clear error
- Accepts `--version v1.2.3` or positional arg; defaults to latest release
- Downloads binary and checksum from GitHub Releases
- Verifies SHA256 checksum
- Installs to `/usr/local/bin` (root) or `~/.local/bin` (non-root)
- Requires `curl` or `wget`, and `sha256sum` or `shasum`

**Usage:**
```bash
# Latest
curl -fsSL https://raw.githubusercontent.com/antstanley/oidc-exchange/main/install.sh | bash

# Specific version
curl -fsSL https://raw.githubusercontent.com/antstanley/oidc-exchange/main/install.sh | bash -s -- --version v1.2.3
```

### Root Dockerfile

Multi-stage build at repo root, replacing the per-example compilation Dockerfiles.

```dockerfile
FROM rust:1.85-slim AS builder
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates curl && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/oidc-exchange /usr/local/bin/
WORKDIR /app
EXPOSE 8080
CMD ["oidc-exchange"]
```

Example Dockerfiles simplified to:
```dockerfile
FROM ghcr.io/antstanley/oidc-exchange:latest
COPY config/ /app/config/
ENV OIDC_EXCHANGE_ENV=container
```

### Binary `--version` flag

Add a `--version` flag to the binary (via `clap` or manual arg parsing) that prints the version from `Cargo.toml` and exits. Required for smoke tests and install script verification.

### Binary Smoke Tests

Run on each platform's native runner after build:

1. Start binary with minimal config (local keys, noop audit, SQLite)
2. `GET /health` → 200
3. `GET /keys` → valid JWKS JSON
4. `GET /.well-known/openid-configuration` → valid OpenID configuration
5. Binary `--version` → expected version string

### Install Script Tests

Run on `ubuntu-latest` and `macos-latest`:

1. Run `install.sh` → binary at expected path
2. `oidc-exchange --version` → correct version
3. Version pinning with a known tag

---

## Sub-project 3: Node.js Bindings

### Package: `@oidc-exchange/node`

Published to npm. Platform-specific native binaries distributed via napi-rs optional dependencies pattern.

### Location

`bindings/nodejs/` — Cargo workspace member.

### Platform packages (optionalDependencies)

| Package | Platform |
|---------|----------|
| `@oidc-exchange/linux-x64-gnu` | Linux x86_64 |
| `@oidc-exchange/linux-arm64-gnu` | Linux arm64 |
| `@oidc-exchange/win32-x64-msvc` | Windows x86_64 |
| `@oidc-exchange/darwin-arm64` | macOS arm64 |

### API

```typescript
export class OidcExchange {
  /**
   * Initialize with a config file path or inline TOML string.
   */
  constructor(options: { config?: string; configString?: string });

  /**
   * Handle a raw HTTP request. Framework-agnostic.
   */
  handleRequest(request: {
    method: string;
    path: string;
    headers: Record<string, string>;
    body?: Buffer | string;
  }): Promise<{
    status: number;
    headers: Record<string, string>;
    body: Buffer;
  }>;

  /**
   * Get a Node.js http.RequestListener for use with http.createServer
   * or any framework accepting (req, res) handlers.
   */
  requestListener(): (req: IncomingMessage, res: ServerResponse) => void;

  /**
   * Graceful shutdown.
   */
  shutdown(): void;
}
```

### Rust side (`bindings/nodejs/src/lib.rs`)

- Uses napi-rs `#[napi]` macros to expose `OidcExchange` class
- Wraps `crates/ffi::OidcExchange`
- `handleRequest` uses napi-rs async task to call `ffi.handle_request()` on the Tokio runtime without blocking the Node.js event loop
- `requestListener` returns a JS function that reads the Node.js `IncomingMessage`, calls `handleRequest`, and writes to `ServerResponse`

### Framework Examples

```
examples/nodejs/
├── express/        # Express app mounting at /auth/*
├── hono/           # Hono app with Web Standard Request/Response
├── fastify/        # Fastify app with raw handler
├── nextjs/         # Next.js route handler (app router)
├── sveltekit/      # SvelteKit server hook
└── serverless/     # Serverless Framework handler
```

Each example contains:
- `package.json` with framework + `@oidc-exchange/node` dependency
- Entry file with minimal mount code
- `config.toml` with local keys + noop adapters
- `README.md` with run instructions

### Tests

- **Unit tests (vitest):** Create `OidcExchange` with noop/local config, test `handleRequest` for health, JWKS, discovery, token exchange (with mock provider), refresh, revoke
- **Integration test:** Full token exchange flow with local key manager and SQLite
- **Type tests:** Verify TypeScript definitions compile correctly

---

## Sub-project 4: Python Bindings

### Package: `oidc-exchange`

Published to PyPI via maturin. Platform-specific wheels contain the native library.

### Location

`bindings/python/` — Cargo workspace member.

### Wheels

| Wheel | Platform |
|-------|----------|
| `oidc_exchange-*-cp310-abi3-manylinux_2_28_x86_64.whl` | Linux x86_64 |
| `oidc_exchange-*-cp310-abi3-manylinux_2_28_aarch64.whl` | Linux arm64 |
| `oidc_exchange-*-cp310-abi3-win_amd64.whl` | Windows x86_64 |
| `oidc_exchange-*-cp310-abi3-macosx_11_0_arm64.whl` | macOS arm64 |

Uses `abi3` stable ABI targeting Python 3.10+ — a single wheel works across Python 3.10, 3.11, 3.12, 3.13.

### API

```python
class OidcExchange:
    def __init__(self, *, config: str | None = None, config_string: str | None = None) -> None:
        """Initialize with a config file path or inline TOML string."""
        ...

    async def handle_request(self, request: dict) -> dict:
        """
        Handle an HTTP request.
        
        Args:
            request: { "method": str, "path": str, "headers": dict[str, str], "body": bytes | str }
        
        Returns:
            { "status": int, "headers": dict[str, str], "body": bytes }
        """
        ...

    def handle_request_sync(self, request: dict) -> dict:
        """Synchronous version of handle_request for WSGI contexts."""
        ...

    def asgi_app(self) -> ASGIApplication:
        """Return an ASGI application that can be mounted in FastAPI/Starlette."""
        ...

    def wsgi_app(self) -> WSGIApplication:
        """Return a WSGI application that can be mounted in Flask/Django."""
        ...

    def shutdown(self) -> None:
        """Graceful shutdown."""
        ...
```

### Rust side (`bindings/python/src/lib.rs`)

- Uses PyO3 `#[pyclass]` and `#[pymethods]` macros
- Wraps `crates/ffi::OidcExchange`
- `handle_request` returns a Python awaitable via `pyo3_asyncio` (or manual future)
- `handle_request_sync` calls `ffi.handle_request()` directly (safe from non-async Python)

### Python-side adapters (`bindings/python/python/oidc_exchange/`)

- `__init__.py` — re-exports `OidcExchange` from the native module
- `_asgi.py` — ASGI adapter: receives ASGI scope/receive/send, translates to `handle_request`, sends response
- `_wsgi.py` — WSGI adapter: receives WSGI environ/start_response, translates to `handle_request_sync`, returns response body
- `py.typed` — PEP 561 marker for type checking
- `__init__.pyi` — Type stubs

### Framework Examples

```
examples/python/
├── fastapi/        # FastAPI app mounting ASGI adapter at /auth
├── flask/          # Flask app mounting WSGI adapter at /auth
└── django/         # Django project with WSGI adapter in urls.py
```

Each example contains:
- `requirements.txt` with framework + `oidc-exchange` dependency
- Entry file with minimal mount code
- `config.toml` with local keys + noop adapters
- `README.md` with run instructions

### Tests

- **Unit tests (pytest):** Create `OidcExchange` with noop/local config, test `handle_request` for health, JWKS, discovery, token exchange, refresh, revoke
- **ASGI tests:** Use httpx `ASGITransport` to test the ASGI adapter directly
- **WSGI tests:** Use werkzeug `Client` to test the WSGI adapter directly
- **Type tests:** mypy against the `.pyi` stubs
- **Integration test:** Full token exchange flow with local key manager and SQLite

---

## Sub-project 5: Documentation & Examples Update

### Website (`apps/website/src/content/docs/`)

**New pages:**

| Page | Content |
|------|---------|
| `getting-started/installation.md` | All install methods: one-line script, prebuilt binary, Docker, npm, pip, cargo install |
| `guides/nodejs.md` | Node.js bindings: install, initialize, framework examples (Express, Hono, Fastify, NextJS, SvelteKit, Serverless) |
| `guides/python.md` | Python bindings: install, initialize, framework examples (FastAPI, Flask, Django) |
| `guides/docker.md` | Docker usage: pulling images, config mounting, compose examples, multi-arch |

**Updated pages:**

| Page | Changes |
|------|---------|
| `getting-started/introduction.md` | Add bindings and Docker to feature list |
| `getting-started/quick-start.md` | Lead with prebuilt binary / Docker instead of building from source |
| `deployment/*.md` | Reference prebuilt images/binaries instead of compiling from source |

### Root `README.md`

- Lead install section with one-line script and Docker
- Add badges: npm, PyPI, Docker Hub, GitHub Release
- Add Node.js and Python quick-start snippets

### Example READMEs

Update `examples/container/`, `examples/ecs-fargate/`, `examples/linux-postgres/`, `examples/linux-sqlite/` READMEs to reference prebuilt Docker images.

### `docs/integration/*.md`

Update all integration guides to reference prebuilt binaries and Docker images.

---

## Version Coordination

All version numbers must match across:

| File | Field |
|------|-------|
| Root `Cargo.toml` | `workspace.package.version` |
| `bindings/nodejs/package.json` | `version` |
| `bindings/python/pyproject.toml` | `project.version` |

The release workflow validates this before building. Version bumps are manual — update all three files, commit, tag, push.

## Tagging Strategy

All published artifacts use consistent semver tags:

| Artifact | Tags |
|----------|------|
| GitHub Release | `v1.0.0` |
| Docker images | `latest`, `v1.0.0`, `v1.0`, `v1` |
| npm package | `1.0.0` (npm convention, no `v` prefix) |
| PyPI package | `1.0.0` (PyPI convention, no `v` prefix) |
