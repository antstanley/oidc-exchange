---
title: "Contributing"
description: "Development setup, testing, and code standards."
---

## Prerequisites

- **Rust:** Install a stable toolchain with [rustup](https://rustup.rs/). Continuous integration (CI) uses rustc 1.98. The workspace does not specify a minimum supported Rust version (MSRV).
- **cargo-nextest:** Use this to run tests. Install with `cargo install cargo-nextest`.
- **cargo-lambda:** Install this if you need to build Lambda binaries. Install with `cargo install cargo-lambda`.
- **Docker:** Install this to run DynamoDB Local integration tests.
- **jj (Jujutsu):** Use this for version control. Install from [martinvonz/jj](https://github.com/martinvonz/jj). This project uses jj exclusively; do not use git CLI commands.

## Clone and build

```bash
jj git clone <repo-url> oidc-exchange
cd oidc-exchange
cargo build
```

Any editor with rust-analyzer support works. The workspace root `Cargo.toml` defines shared dependency versions.

## Version control with jj

This project uses **Jujutsu (jj)**, a Git-compatible version control system. It records working-copy changes automatically and has no staging area.

### Common workflows

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

### Bookmarks (branching)

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

- **No staging area:** All file edits become part of the current change automatically.
- **Immutable commits**: `jj describe`, `jj squash`, and `jj rebase` create new commit IDs. This is safe; jj tracks the rewrite.
- **Conflict markers in files**: jj allows conflicted states to exist in the working copy. Resolve conflicts, then `jj status` confirms resolution.
- **`jj new` instead of `git commit`:** When your current change is ready, run `jj new` to start a fresh change on top of it.

## Testing

### Test runner

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

### Integration tests

Some adapter tests require external services and are marked `#[ignore]`. To run them:

```bash
# Start DynamoDB Local
docker run -d -p 8000:8000 amazon/dynamodb-local

# Run ignored tests
cargo nextest run -p oidc-exchange-adapters -- --ignored
```

### Test architecture

The codebase separates business logic from infrastructure through interfaces called ports. This lets tests replace infrastructure with mock implementations:

- **`crates/test-utils/`**: provides mock implementations of all port traits (`MockRepository`, `MockKeyManager`, `MockAuditLog`, `MockRateLimiter`, `MockIdentityProvider`, `MockUserSync`). These are in-memory implementations used by core service tests and server E2E tests.
- **Core tests** (`crates/core/tests/`) test business logic with mocks. They do not use the network or filesystem.
- **Adapter tests** (`crates/adapters/tests/`) test infrastructure integrations. HTTP-based adapters use [wiremock](https://crates.io/crates/wiremock) for deterministic HTTP mocking. DynamoDB tests require DynamoDB Local.
- **Server E2E tests** (`crates/server/tests/`) start an axum router with mock adapters and send HTTP requests.

### Writing tests

- Place unit tests in the module they test (standard Rust `#[cfg(test)]` blocks).
- Place integration tests in the crate's `tests/` directory.
- Use the mock implementations from `test-utils`; do not duplicate mock logic.
- Tests that need external services must be `#[ignore]` so the default `cargo nextest run` works without Docker.

## Code organization

### Crate structure

| Crate | Package name | Purpose |
|---|---|---|
| `crates/core` | `oidc-exchange-core` | Domain types, port traits, service logic. Zero infrastructure dependencies. |
| `crates/adapters` | `oidc-exchange-adapters` | Implementations of port traits for DynamoDB, KMS, SQS, OIDC, webhooks. |
| `crates/providers` | `oidc-exchange-providers` | Non-standard identity provider modules (Apple). |
| `crates/server` | `oidc-exchange` | HTTP layer (axum), middleware, telemetry, and the binary entrypoint. |
| `crates/ffi` | `oidc-exchange-ffi` | Language-agnostic request/response FFI wrapper over the server router. Reused by the Node.js and Python bindings. |
| `crates/test-utils` | `oidc-exchange-test-utils` | Mock implementations of all ports. Dev-dependency only. |

### Dependency rules

- `core` depends on nothing infrastructure-specific (no AWS SDKs, no HTTP clients).
- `adapters` and `providers` depend on `core` for trait definitions.
- `server` depends on `core`, `adapters`, and `providers`.
- `ffi` depends on `server` (router construction) and `core`; the Node.js and Python bindings depend on `ffi`.
- `test-utils` depends only on `core`.

The crate dependencies keep core logic separate from infrastructure.

### Adding a new adapter

1. Define the implementation in `crates/adapters/src/`.
2. Implement the relevant port trait from `crates/core/src/ports/`.
3. Add a builder function (e.g., `from_config()`) that constructs the adapter from the TOML config.
4. Wire it into the adapter selection in `crates/server/src/bootstrap.rs`.
5. Add tests. Use wiremock for HTTP adapters and Docker services for database adapters.

### Adding a new identity provider

1. If the provider follows standard OIDC, it only needs a config entry, with no code required.
2. If the provider has specific requirements, such as Apple, add a module in `crates/providers/src/` implementing `IdentityProvider`.
3. Add an adapter name and register it in provider construction in `crates/server/src/bootstrap.rs`.

## Code standards

### Writing guidelines

These guidelines apply to documentation, code comments, commit and pull request messages, and replies to users.

- Write clear, complete sentences. Keep the detail proportional to the task.
- Remove filler, repetition, and unrelated details.
- Follow the plain language principles of ISO 24495-1:2023. Use short sentences with one idea each. Define terms on first use.
- Do not use em dashes or teasers such as "Here's the thing" or "But there's a catch".

### Formatting and linting

```bash
# Format
cargo fmt --all

# Lint
cargo clippy --workspace --all-targets
```

Both must pass with zero warnings before pushing.

### Error handling

- Use `thiserror` for error enums. All domain errors are in `crates/core/src/error.rs`.
- Return domain errors from service methods. The server crate maps these to HTTP responses.
- Do not use `.unwrap()` outside of tests.

### Configuration

- New config fields go in `crates/core/src/config.rs` as strongly-typed structs with serde.
- All secrets use `${VAR_NAME}` placeholder syntax; never hardcode secrets in TOML defaults.
- New config sections need a corresponding entry in `config/default.toml`.

### Commit messages

Write concise commit messages that describe what changed and why. Use `jj describe` to set the current change's description.

```
fix: reject expired refresh tokens before database lookup

The previous flow queried the database before checking expiry.
Repeated requests with expired tokens added unnecessary load.
```

Prefix with a type when it helps clarity: `fix:`, `feat:`, `refactor:`, `test:`, `docs:`, `chore:`.

## Running the full stack locally

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

The server starts on `http://localhost:8080`. Use the `/health` endpoint to verify it is running.
