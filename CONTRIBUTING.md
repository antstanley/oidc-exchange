# Contributing Guide

## Development Setup

### Prerequisites

- **Rust** — stable toolchain, 1.75 or later. Install via [rustup](https://rustup.rs/).
- **cargo-nextest** — test runner. Install with `cargo install cargo-nextest`.
- **cargo-lambda** — required only for building Lambda binaries. Install with `cargo install cargo-lambda`.
- **Docker** — required for DynamoDB Local integration tests.
- **jj (Jujutsu)** — version control. Install from [martinvonz/jj](https://github.com/martinvonz/jj). This project uses jj exclusively; do not use git CLI commands.
- **Node.js 22+** — required for Node.js bindings development.
- **pnpm** — Node.js package manager. Install with `npm install -g pnpm`.
- **Python 3.10+** — required for Python bindings development.
- **uv** — Python package manager. Install from [astral.sh/uv](https://docs.astral.sh/uv/).

### Clone and build

```bash
jj git clone <repo-url> oidc-exchange
cd oidc-exchange
cargo build
```

### Editor setup

The workspace is a standard Cargo workspace. Any editor with rust-analyzer support works. The workspace root `Cargo.toml` defines shared dependency versions.

## Version Control

This project uses **Jujutsu (jj)** for version control. jj is a Git-compatible VCS with a simpler mental model — there is no staging area, and every working-copy state is automatically committed.

**Always use `jj` for version control. Never use `git` CLI commands directly.**

### Standard workflow

For every change:

1. Create a new change: `jj new`
2. Make your edits
3. Describe the change: `jj describe -m "feat: add domain allowlist validation"`
4. Set the main bookmark: `jj bookmark set main`
5. Push: `jj git push`

### Common commands

```bash
# See status
jj status

# Describe the current change
jj describe -m "add domain allowlist validation"

# Create a new empty change on top of the current one
jj new

# View the log
jj log

# Push to remote
jj git push
```

### Bookmarks (branches)

jj uses bookmarks instead of branches:

```bash
# Create a bookmark
jj bookmark create my-feature

# Move a bookmark to the current change
jj bookmark set my-feature

# Push a bookmark
jj git push --bookmark my-feature
```

### Key differences from git

- **No staging area** — all file changes are part of the current change automatically.
- **Immutable commits** — `jj describe`, `jj squash`, and `jj rebase` create new commit IDs. This is safe; jj tracks the rewrite.
- **Conflict markers in files** — jj allows conflicted states to exist in the working copy. Resolve conflicts, then `jj status` confirms resolution.
- **`jj new` instead of `git commit`** — when your current change is ready, run `jj new` to start a fresh change on top of it.

## Language-Specific Standards

### Rust

| Concern | Tool | Command |
|---------|------|---------|
| Formatting | `rustfmt` | `cargo fmt --all` |
| Linting | `clippy` | `cargo clippy --workspace -- -D warnings` |
| Testing | `cargo-nextest` | `cargo nextest run --workspace` |

All Rust code must pass `cargo fmt --check --all` and `cargo clippy --workspace -- -D warnings` with zero warnings before pushing.

### Node.js / TypeScript

| Concern | Tool | Command |
|---------|------|---------|
| Language | TypeScript | All code must be TypeScript (`.ts`) |
| Module system | ES Modules | All packages use `"type": "module"` |
| Package manager | `pnpm` | `pnpm install` |
| Formatting | `oxfmt` | `pnpm fmt` / `pnpm fmt:check` |
| Linting | `oxlint` | `pnpm lint` |
| Testing | `vitest` | `pnpm test` |

**All JavaScript/Node.js code must be TypeScript.** No plain `.js` source files (except generated output). Use `.ts` for all source code.

**All packages must use ES Modules exclusively.** Every `package.json` must include `"type": "module"`. Never use `require()` or `module.exports` — use `import`/`export` instead. The only exception is `createRequire()` from `node:module` for loading native `.node` addons.

**Always use `pnpm`** — never `npm` or `yarn`. The `pnpm-lock.yaml` is the lockfile of record.

**Always use `oxfmt`** for formatting TypeScript, JSON, and related files.

**Always use `oxlint`** for linting TypeScript.

**Always use `vitest`** for testing. Tests live in `__tests__/` directories or alongside source files with `.test.ts` extensions.

### Python

| Concern | Tool | Command |
|---------|------|---------|
| Package manager | `uv` | `uv sync` / `uv add <pkg>` |
| Formatting | `ruff format` | `uv run ruff format .` |
| Linting | `ruff check` | `uv run ruff check .` |
| Type validation | `pydantic` | Use for data models requiring validation |
| Testing | `pytest` | `uv run pytest` |

**Always use `uv`** for Python package management — never `pip`, `pip install`, or manual virtualenvs. `uv` manages the virtualenv automatically.

**Always use `ruff`** for both formatting and linting Python code.

**Always use `pydantic`** for types and validation where structured data validation is required.

**Always use `pytest`** for testing. Tests live in `tests/` directories.

## Testing

### Rust tests

All tests run through [cargo-nextest](https://nexte.st/), configured in `.config/nextest.toml`.

```bash
# Run the full test suite
cargo nextest run --workspace

# Run tests for a specific crate
cargo nextest run -p oidc-exchange-core
cargo nextest run -p oidc-exchange-adapters
cargo nextest run -p oidc-exchange        # server crate

# Run a single test by name
cargo nextest run --workspace -E 'test(exchange_valid_code)'

# Use the CI profile (stricter: 2 retries, fail-fast)
cargo nextest run --workspace --profile ci
```

### Node.js tests

```bash
cd bindings/nodejs
pnpm test          # runs vitest
```

### Python tests

```bash
cd bindings/python
uv run pytest      # runs pytest via uv-managed virtualenv
```

### Integration tests

Some adapter tests require external services and are marked `#[ignore]`. To run them:

```bash
# Start DynamoDB Local
docker run -d -p 8000:8000 amazon/dynamodb-local

# Run ignored tests
cargo nextest run -p oidc-exchange-adapters -- --ignored
```

### Test architecture

The codebase uses the hexagonal architecture to make testing straightforward:

- **`crates/test-utils/`** — provides mock implementations of all port traits (`MockRepository`, `MockKeyManager`, `MockAuditLog`, `MockIdentityProvider`, `MockUserSync`). These are in-memory implementations used by core service tests and server E2E tests.
- **Core tests** (`crates/core/tests/`) — test business logic in isolation using mocks. No network, no filesystem.
- **Adapter tests** (`crates/adapters/tests/`) — test infrastructure integrations. HTTP-based adapters use [wiremock](https://crates.io/crates/wiremock) for deterministic HTTP mocking. DynamoDB tests require DynamoDB Local.
- **Server E2E tests** (`crates/server/tests/`) — spin up a full axum router with mock adapters and issue real HTTP requests.

### Writing tests

- Place unit tests in the module they test (standard Rust `#[cfg(test)]` blocks).
- Place integration tests in the crate's `tests/` directory.
- Use the mock implementations from `test-utils` — do not duplicate mock logic.
- Tests that need external services must be `#[ignore]` so the default `cargo nextest run` works without Docker.

## Code Organization

### Crate structure

| Crate | Package name | Purpose |
|-------|-------------|---------|
| `crates/core` | `oidc-exchange-core` | Domain types, port traits, service logic. Zero infrastructure dependencies. |
| `crates/adapters` | `oidc-exchange-adapters` | Implementations of port traits for DynamoDB, KMS, CloudTrail, OIDC, webhooks. |
| `crates/providers` | `oidc-exchange-providers` | Non-standard identity provider modules (Apple). |
| `crates/server` | `oidc-exchange` | HTTP layer (axum), middleware, telemetry, bootstrap, and the binary entrypoint. |
| `crates/ffi` | `oidc-exchange-ffi` | FFI wrapper for language bindings. Wraps AppService + Axum router. |
| `crates/test-utils` | `oidc-exchange-test-utils` | Mock implementations of all ports. Dev-dependency only. |
| `bindings/nodejs` | `@oidc-exchange/node` | Node.js bindings via napi-rs. |
| `bindings/python` | `oidc-exchange` (PyPI) | Python bindings via PyO3/maturin. |

### Dependency rules

- `core` depends on nothing infrastructure-specific (no AWS SDKs, no HTTP clients).
- `adapters` and `providers` depend on `core` for trait definitions.
- `server` depends on `core`, `adapters`, and `providers`.
- `ffi` depends on `server` (for bootstrap module), `core`, `adapters`, and `providers`.
- `test-utils` depends only on `core`.
- `bindings/nodejs` and `bindings/python` depend only on `ffi`.

These boundaries are enforced by the Cargo workspace. If `core` compiles, the domain logic is free of infrastructure coupling.

### Adding a new adapter

1. Define the implementation in `crates/adapters/src/`.
2. Implement the relevant port trait from `crates/core/src/ports/`.
3. Add a builder function (e.g., `from_config()`) that constructs the adapter from the TOML config.
4. Wire it into the adapter selection in `crates/server/src/bootstrap.rs`.
5. Add tests — use wiremock for HTTP-based adapters, Docker services for database adapters.

### Adding a new identity provider

1. If the provider follows standard OIDC, it only needs a config entry — no code.
2. If the provider has quirks (like Apple), add a module in `crates/providers/src/` implementing `IdentityProvider`.
3. Add an adapter name and wire it into provider construction in `crates/server/src/bootstrap.rs`.

## Documentation

The canonical source for all documentation is the `docs/` directory at the repo root. The website at `apps/website/` reads from `docs/` via a symlink (`apps/website/src/content/docs` → `docs/`).

**Always edit files in `docs/`** — never edit content directly in `apps/website/src/content/docs/`. Changes to `docs/` automatically appear on the website.

### Structure

```
docs/
├── getting-started/     # Introduction, installation, quick-start
├── guides/              # Configuration, providers, API reference, Node.js, Python, Docker
├── deployment/          # AWS Lambda, ECS Fargate, container, Linux server scenarios
├── architecture/        # Architecture overview, adapter documentation
├── contributing/        # Contributing guide (website version)
└── superpowers/         # Internal design specs and implementation plans (not user-facing)
```

### Code examples in docs

Code examples shown in documentation pages must match the actual code in the `examples/` directory. When updating an example, update both the `examples/` code and the corresponding documentation page.

## Code Standards

### Formatting and linting

```bash
# Rust
cargo fmt --all
cargo clippy --workspace -- -D warnings

# Node.js
cd bindings/nodejs && pnpm fmt && pnpm lint

# Python
cd bindings/python && uv run ruff format . && uv run ruff check .
```

All must pass with zero warnings/errors before pushing.

### Error handling

- Use `thiserror` for error enums. All domain errors are in `crates/core/src/error.rs`.
- Return domain errors from service methods. The server crate maps these to HTTP responses.
- Do not use `.unwrap()` outside of tests.

### Configuration

- New config fields go in `crates/core/src/config.rs` as strongly-typed structs with serde.
- All secrets use `${VAR_NAME}` placeholder syntax — never hardcode secrets in TOML defaults.
- New config sections need a corresponding entry in `config/default.toml`.

### Commit messages

Write concise commit messages that describe *what* changed and *why*. Use `jj describe` to set the current change's description.

```
fix: reject expired refresh tokens before database lookup

The previous flow hit the database before checking expiry,
adding unnecessary load during token storms.
```

Prefix with a type when it helps clarity: `fix:`, `feat:`, `refactor:`, `test:`, `docs:`, `chore:`.

## Running the Full Stack Locally

The `examples/aws-web/` directory contains a complete demo application (SvelteKit + CDK). For local development without AWS:

1. Start DynamoDB Local:

   ```bash
   docker run -d -p 8000:8000 amazon/dynamodb-local
   ```

2. Generate a local signing key:

   ```bash
   openssl genpkey -algorithm ed25519 -out keys/dev.pem
   ```

3. Create `config/local.toml` with `adapter = "local"` for key manager and DynamoDB pointed at `http://localhost:8000`.

4. Run:

   ```bash
   OIDC_EXCHANGE_ENV=local cargo run
   ```

The server starts on `http://localhost:8080`. Use the `/health` endpoint to verify it's running.
