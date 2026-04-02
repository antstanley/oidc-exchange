# Distribution & Language Bindings Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship prebuilt binaries, Docker images, an install script, and FFI bindings for Node.js and Python so users can adopt oidc-exchange without compiling from source.

**Architecture:** A new `crates/ffi` Rust crate wraps the existing `AppService` + Axum router into a synchronous `handle_request(method, path, headers, body) -> (status, headers, body)` interface. Node.js bindings (napi-rs) and Python bindings (PyO3/maturin) both consume this crate. GitHub Actions workflows build/release all artifacts on semver tag push. The existing `main.rs` adapter-building logic is extracted into a shared `bootstrap` module so both `main.rs` and `crates/ffi` can reuse it.

**Tech Stack:** Rust (axum, tokio), napi-rs (Node.js FFI), PyO3 + maturin (Python FFI), GitHub Actions, Docker buildx, cross-rs (arm64 Linux)

**Version control:** Use `jj` for all operations. Each task: `jj new` before starting, `jj describe` when done, `jj bookmark set main` then `jj git push` after each change.

**Spec:** `docs/superpowers/specs/2026-04-02-distribution-and-bindings-design.md`

---

## Sub-project 1: FFI Crate

### Task 1.1: Extract bootstrap module from main.rs

**Files:**
- Create: `crates/server/src/bootstrap.rs`
- Modify: `crates/server/src/lib.rs`
- Modify: `crates/server/src/main.rs`

The goal is to move all adapter-building functions and config loading out of `main.rs` into a reusable `bootstrap` module. The functions to extract: `load_config`, `build_user_repository`, `build_session_repository`, `build_key_manager`, `build_audit_log`, `build_user_sync`, `build_providers`, `build_single_provider`, `provider_config_to_oidc`, `build_dynamo_client`. Also add two new public functions: `parse_config(toml_str) -> AppConfig` and `build_router(config, service) -> Router`.

- [ ] **Step 1: Create `crates/server/src/bootstrap.rs`**

Read `crates/server/src/main.rs` fully. Move all functions except `main()` into bootstrap.rs. Add `pub fn parse_config(toml_str: &str) -> Result<AppConfig, Box<dyn std::error::Error>>` that parses a TOML string to AppConfig. Add `pub fn build_router(config: &AppConfig, service: AppService) -> Router` that takes the router-construction logic from main (lines 71-86), builds `AppState`, merges routes based on role, applies middleware, and returns the final `Router`. Keep `build_service` as `pub async fn build_service(config: &AppConfig) -> Result<AppService, Box<dyn std::error::Error>>`. All adapter builders stay private.

- [ ] **Step 2: Add `pub mod bootstrap;` to `crates/server/src/lib.rs`**

- [ ] **Step 3: Simplify main.rs to use bootstrap**

Replace main.rs body with: `let config = bootstrap::load_config()?; telemetry::init_telemetry(...); let service = bootstrap::build_service(&config).await?; let app = bootstrap::build_router(&config, service);` then the listener/serve logic.

- [ ] **Step 4: Run `cargo nextest run --workspace`** to verify refactor

- [ ] **Step 5: Commit with jj**

`jj new && jj describe -m "refactor: extract bootstrap module from main.rs for reuse by FFI crate" && jj bookmark set main && jj git push`

---

### Task 1.2: Create FFI crate

**Files:**
- Modify: `Cargo.toml` (workspace root) - add `"crates/ffi"` to members
- Create: `crates/ffi/Cargo.toml`
- Create: `crates/ffi/src/lib.rs`

- [ ] **Step 1: Add `"crates/ffi"` to workspace members in root Cargo.toml**

- [ ] **Step 2: Create `crates/ffi/Cargo.toml`**

Dependencies: `oidc-exchange` (path ../server), `oidc-exchange-core` (workspace), `tokio` (workspace), `axum` 0.8, `http` 1, `http-body-util` 0.1, `tower` 0.5 with "util" feature, `tracing` (workspace). Dev-deps: `oidc-exchange-test-utils` (workspace), `serde_json` (workspace), `ed25519-dalek` 3.0.0-pre.6 with pkcs8 feature, `pem` 3.

- [ ] **Step 3: Create `crates/ffi/src/lib.rs`**

Define `FfiError { code: String, message: String }` (impl Display + Error), `FfiResponse { status: u16, headers: Vec<(String, String)>, body: Vec<u8> }`, and `OidcExchange { runtime: tokio::runtime::Runtime, router: axum::Router }`.

Implement:
- `OidcExchange::new(config_toml: &str) -> Result<Self, FfiError>` - creates Runtime, calls `bootstrap::parse_config`, `runtime.block_on(bootstrap::build_service(&config))`, `bootstrap::build_router`.
- `OidcExchange::from_file(path: &str) -> Result<Self, FfiError>` - reads file, delegates to `new`.
- `OidcExchange::handle_request(&self, method, path, headers, body) -> Result<FfiResponse, FfiError>` - builds `http::Request`, clones router, calls `self.runtime.block_on(async { router.oneshot(request).await })`, collects response body via `BodyExt::collect`.

- [ ] **Step 4: Run `cargo build -p oidc-exchange-ffi`**

- [ ] **Step 5: Commit with jj**

---

### Task 1.3: Add FFI crate tests

**Files:**
- Create: `crates/ffi/tests/integration.rs`

- [ ] **Step 1: Write tests**

Tests need a helper `setup_test_key()` that generates an Ed25519 PEM using `ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])` and writes PKCS#8 PEM to `test-key.pem`. Use `minimal_config()` returning a TOML string with sqlite :memory:, local key manager pointing to test-key.pem, noop audit, issuer "https://auth.test.com".

Tests: `test_health_endpoint` (GET /health -> 200), `test_jwks_endpoint` (GET /keys -> 200, body has "keys" array), `test_openid_discovery` (GET /.well-known/openid-configuration -> 200, issuer matches), `test_invalid_config` (bad TOML -> FfiError with code "config_error"), `test_invalid_method` (NOTAMETHOD -> FfiError). Clean up test-key.pem in each test.

- [ ] **Step 2: Run `cargo nextest run -p oidc-exchange-ffi`**

- [ ] **Step 3: Commit with jj**

---

## Sub-project 2: Binary Distribution and Docker

### Task 2.1: Add --version flag to binary

**Files:**
- Modify: `crates/server/src/main.rs`

- [ ] **Step 1: Add version flag**

Add `const VERSION: &str = env!("CARGO_PKG_VERSION");` at top. Before config loading, check `std::env::args().any(|a| a == "--version" || a == "-V")`, if true print `oidc-exchange {VERSION}` and return Ok(()).

- [ ] **Step 2: Run `cargo run -p oidc-exchange -- --version`**, expect `oidc-exchange 0.1.0`

- [ ] **Step 3: Commit with jj**

---

### Task 2.2: Create root Dockerfile

**Files:**
- Create: `Dockerfile`
- Modify: `examples/container/Dockerfile`
- Modify: `examples/ecs-fargate/Dockerfile`

- [ ] **Step 1: Write root Dockerfile** - Multi-stage: `rust:1.85-slim` builder with `pkg-config libssl-dev`, cargo build --release --bin oidc-exchange. Runtime: `debian:bookworm-slim` with ca-certificates curl.

- [ ] **Step 2: Simplify example Dockerfiles** to `FROM ghcr.io/antstanley/oidc-exchange:latest` + COPY config + ENV.

- [ ] **Step 3: Commit with jj**

---

### Task 2.3: Create install script

**Files:**
- Create: `install.sh`

- [ ] **Step 1: Write install.sh**

Bash script. Detects OS (uname -s: Linux/Darwin), arch (uname -m: x86_64/aarch64/arm64). Maps to binary name. Accepts `--version v1.2.3` arg, defaults to latest via GitHub API. Downloads binary + .sha256 from GitHub Releases. Verifies checksum with sha256sum or shasum. Installs to /usr/local/bin (root) or ~/.local/bin (non-root). chmod +x.

- [ ] **Step 2: `chmod +x install.sh`**

- [ ] **Step 3: Commit with jj**

---

### Task 2.4: Create CI workflow

**Files:**
- Create: `.github/workflows/ci.yml`

- [ ] **Step 1: Write CI workflow**

Trigger: push to main, PRs. Jobs: lint (fmt --check, clippy), test (nextest), nodejs-test (npm install + napi build + npm test in bindings/nodejs), python-test (maturin develop + pytest in bindings/python). Use actions/checkout@v4, dtolnay/rust-toolchain@stable, Swatinem/rust-cache@v2, actions/setup-node@v4 node 22, actions/setup-python@v5 python 3.10.

- [ ] **Step 2: Commit with jj**

---

### Task 2.5: Create release workflow

**Files:**
- Create: `.github/workflows/release.yml`

- [ ] **Step 1: Write release workflow**

Trigger: push tags `v*.*.*`. Jobs: validate (check versions match across Cargo.toml, package.json, pyproject.toml), build-binaries (matrix 4 targets using cross for arm64 linux), build-docker (buildx multi-arch push to ghcr.io + Docker Hub with semver tags), build-nodejs (napi-rs matrix), publish-nodejs, build-python (maturin matrix), publish-python, create-release (gh-release with changelog). Full YAML as specified in the detailed plan content.

- [ ] **Step 2: Commit with jj**

---

## Sub-project 3: Node.js Bindings

### Task 3.1: Create Node.js binding crate and package

**Files:**
- Modify: `Cargo.toml` - add `"bindings/nodejs"` to workspace members
- Create: `bindings/nodejs/Cargo.toml` - cdylib, deps: oidc-exchange-ffi, napi 2 (async, napi9), napi-derive 2
- Create: `bindings/nodejs/build.rs` - napi_build::setup()
- Create: `bindings/nodejs/src/lib.rs` - napi wrapper
- Create: `bindings/nodejs/package.json` - @oidc-exchange/node
- Create: `bindings/nodejs/index.js` - platform detection loader
- Create: `bindings/nodejs/index.d.ts` - TypeScript types

- [ ] **Step 1: Add to workspace and create Cargo.toml + build.rs**

- [ ] **Step 2: Create src/lib.rs**

Define napi objects: `HttpRequest { method, path, headers: Vec<HeaderEntry>, body: Option<Buffer> }`, `HeaderEntry { name, value }`, `HttpResponse { status: u32, headers: Vec<HeaderEntry>, body: Buffer }`, `OidcExchangeOptions { config: Option<String>, config_string: Option<String> }`.

`OidcExchange` class with `#[napi(constructor)]` taking OidcExchangeOptions, `#[napi] handle_request` that converts types and calls `self.inner.handle_request`, `#[napi] shutdown` no-op.

- [ ] **Step 3: Create package.json**

Name `@oidc-exchange/node`, version 0.1.0, napi triples for 4 platforms, engines node >= 22, scripts: build (napi build --release), test (node --test __tests__/index.test.mjs).

- [ ] **Step 4: Create index.js** - Platform detection, tries require(platform-package), falls back to local .node file.

- [ ] **Step 5: Create index.d.ts** - TypeScript interfaces matching the napi objects.

- [ ] **Step 6: Run `cd bindings/nodejs && npm install && npx napi build --release`**

- [ ] **Step 7: Commit with jj**

---

### Task 3.2: Create Node.js platform packages

**Files:**
- Create: `bindings/nodejs/npm/linux-x64-gnu/package.json`
- Create: `bindings/nodejs/npm/linux-arm64-gnu/package.json`
- Create: `bindings/nodejs/npm/win32-x64-msvc/package.json`
- Create: `bindings/nodejs/npm/darwin-arm64/package.json`

- [ ] **Step 1: Create 4 platform package.json files** - Each has name (@oidc-exchange/{platform}), version 0.1.0, os, cpu, main pointing to .node file, license MIT.

- [ ] **Step 2: Commit with jj**

---

### Task 3.3: Add Node.js tests

**Files:**
- Create: `bindings/nodejs/__tests__/index.test.mjs`

- [ ] **Step 1: Write tests using node:test**

Setup: generate Ed25519 key via openssl. Tests: create instance from configString, reject missing config, GET /health -> 200, GET /keys -> JWKS JSON with keys array, GET /.well-known/openid-configuration -> issuer matches, GET /nonexistent -> 404.

- [ ] **Step 2: Run `cd bindings/nodejs && npm test`**

- [ ] **Step 3: Commit with jj**

---

### Task 3.4: Create Node.js framework examples

**Files:**
- Create: `examples/nodejs/config.toml` - shared minimal config
- Create: `examples/nodejs/express/` - package.json, index.js, README.md
- Create: `examples/nodejs/hono/` - package.json, index.ts, README.md
- Create: `examples/nodejs/fastify/` - package.json, index.js, README.md
- Create: `examples/nodejs/nextjs/` - package.json, app/auth/[...path]/route.ts, README.md
- Create: `examples/nodejs/sveltekit/` - package.json, src/hooks.server.ts, README.md
- Create: `examples/nodejs/serverless/` - package.json, handler.js, serverless.yml, README.md

- [ ] **Step 1: Create shared config.toml** with sqlite, local keys, noop audit.

- [ ] **Step 2: Create Express example** - Express app.all('/auth/*') that strips prefix, converts headers, reads body chunks, calls handleRequest.

- [ ] **Step 3: Create Hono example** - Hono app.all('/auth/*') using Web Standard Request/Response.

- [ ] **Step 4: Create Fastify example** - Fastify route handler.

- [ ] **Step 5: Create Next.js example** - App Router catch-all route handler at app/auth/[...path]/route.ts.

- [ ] **Step 6: Create SvelteKit example** - Server hook in src/hooks.server.ts.

- [ ] **Step 7: Create Serverless Framework example** - Lambda handler + serverless.yml.

- [ ] **Step 8: Commit with jj**

---

## Sub-project 4: Python Bindings

### Task 4.1: Create Python binding crate and package

**Files:**
- Modify: `Cargo.toml` - add `"bindings/python"` to workspace members
- Create: `bindings/python/Cargo.toml` - cdylib named `_oidc_exchange`, deps: oidc-exchange-ffi, pyo3 0.22 (extension-module, abi3-py310)
- Create: `bindings/python/pyproject.toml` - maturin build, project name oidc-exchange, version 0.1.0, python >= 3.10
- Create: `bindings/python/src/lib.rs` - PyO3 wrapper
- Create: `bindings/python/python/oidc_exchange/__init__.py` - Python package wrapping native module
- Create: `bindings/python/python/oidc_exchange/__init__.pyi` - type stubs
- Create: `bindings/python/python/oidc_exchange/py.typed` - PEP 561 marker

- [ ] **Step 1: Add to workspace and create Cargo.toml + pyproject.toml**

- [ ] **Step 2: Create src/lib.rs**

PyO3 class `OidcExchange` with `#[new]` taking keyword-only `config` and `config_string` options. Method `handle_request_sync` taking Python dict with method/path/headers/body keys, returning Python dict with status/headers/body. `shutdown` no-op. Module function `_oidc_exchange` adding the class.

- [ ] **Step 3: Create Python package files**

`__init__.py`: Import `_OidcExchange` from native module, define `OidcExchange` class wrapping it with `handle_request_sync`, `async handle_request` (using `loop.run_in_executor`), `asgi_app()`, `wsgi_app()`, `shutdown()`. Import `make_asgi_app` and `make_wsgi_app` from submodules.

`__init__.pyi`: Type stubs for the OidcExchange class.

`py.typed`: Empty file.

- [ ] **Step 4: Run `cd bindings/python && pip install maturin && maturin develop`**

- [ ] **Step 5: Commit with jj**

---

### Task 4.2: Create ASGI and WSGI adapters

**Files:**
- Create: `bindings/python/python/oidc_exchange/_asgi.py`
- Create: `bindings/python/python/oidc_exchange/_wsgi.py`

- [ ] **Step 1: Create _asgi.py**

`make_asgi_app(oidc)` returns an async ASGI callable. Reads body from receive loop, builds headers dict from scope headers (decode latin-1), constructs request dict, calls `await oidc.handle_request(request)`, sends http.response.start + http.response.body.

- [ ] **Step 2: Create _wsgi.py**

`make_wsgi_app(oidc)` returns a WSGI callable. Reads body from wsgi.input using CONTENT_LENGTH, builds headers from HTTP_ environ keys, calls `oidc.handle_request_sync(request)`, calls start_response with status line and headers, returns [body].

- [ ] **Step 3: Commit with jj**

---

### Task 4.3: Add Python tests

**Files:**
- Create: `bindings/python/tests/test_handle_request.py`
- Create: `bindings/python/tests/test_asgi.py`
- Create: `bindings/python/tests/test_wsgi.py`

- [ ] **Step 1: Write handle_request tests**

Session-scoped fixture generating Ed25519 test key via openssl. Tests: create instance, reject missing config, GET /health -> 200, GET /keys -> JWKS, GET /.well-known/openid-configuration -> issuer, GET /nonexistent -> 404, async handle_request for /health.

- [ ] **Step 2: Write ASGI tests** using httpx ASGITransport - GET /health, GET /keys.

- [ ] **Step 3: Write WSGI tests** using werkzeug.test.Client - GET /health, GET /keys.

- [ ] **Step 4: Run `cd bindings/python && maturin develop && pytest tests/ -v`**

- [ ] **Step 5: Commit with jj**

---

### Task 4.4: Create Python framework examples

**Files:**
- Create: `examples/python/config.toml` - shared config
- Create: `examples/python/fastapi/` - requirements.txt, main.py, README.md
- Create: `examples/python/flask/` - requirements.txt, app.py, README.md
- Create: `examples/python/django/` - requirements.txt, myproject/urls.py, myproject/settings.py, manage.py, README.md

- [ ] **Step 1: Create shared config.toml**

- [ ] **Step 2: Create FastAPI example** - `app.mount("/auth", oidc.asgi_app())`

- [ ] **Step 3: Create Flask example** - `DispatcherMiddleware(app.wsgi_app, {"/auth": oidc.wsgi_app()})`

- [ ] **Step 4: Create Django example** - catch-all view calling `handle_request_sync` + URL pattern

- [ ] **Step 5: Commit with jj**

---

## Sub-project 5: Documentation

### Task 5.1: Create installation guide

**Files:**
- Create: `apps/website/src/content/docs/getting-started/installation.md`

- [ ] **Step 1: Write installation guide** covering: one-line script, Docker (both registries), npm, pip, prebuilt binary download table (4 platforms), from source.

- [ ] **Step 2: Commit with jj**

---

### Task 5.2: Create Node.js, Python, and Docker guides

**Files:**
- Create: `apps/website/src/content/docs/guides/nodejs.md`
- Create: `apps/website/src/content/docs/guides/python.md`
- Create: `apps/website/src/content/docs/guides/docker.md`

- [ ] **Step 1: Write Node.js guide** - install, basic usage, framework examples (Express, Hono, Fastify, Next.js, SvelteKit), config options.

- [ ] **Step 2: Write Python guide** - install, basic usage, framework examples (FastAPI, Flask, Django), async support, config options.

- [ ] **Step 3: Write Docker guide** - pull, run, compose, multi-arch, tags table, custom Dockerfile.

- [ ] **Step 4: Commit with jj**

---

### Task 5.3: Update existing documentation

**Files:**
- Modify: `README.md` - add badges (GitHub Release, npm, PyPI, Docker), install section with one-line script/Docker/npm/pip
- Modify: `apps/website/src/content/docs/getting-started/introduction.md` - add bindings and Docker to features
- Modify: `apps/website/src/content/docs/getting-started/quick-start.md` - lead with prebuilt install

- [ ] **Step 1: Update README.md** - read current content, add badges after title, add Install section.

- [ ] **Step 2: Update introduction.md** - read and add Node.js/Python/Docker to feature list.

- [ ] **Step 3: Update quick-start.md** - read and change primary path from source build to prebuilt.

- [ ] **Step 4: Commit with jj**

---

### Task 5.4: Update example READMEs and integration docs

**Files:**
- Modify: `examples/container/README.md`
- Modify: `examples/ecs-fargate/README.md`
- Modify: `examples/linux-postgres/README.md`
- Modify: `examples/linux-sqlite/README.md`
- Modify: `docs/integration/*.md`

- [ ] **Step 1: Update example READMEs** - add tip about prebuilt Docker images.

- [ ] **Step 2: Update integration docs** - add references to prebuilt binaries and Docker.

- [ ] **Step 3: Commit with jj**
