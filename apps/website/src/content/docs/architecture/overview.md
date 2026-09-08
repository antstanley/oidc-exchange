---
title: "Architecture"
description: "Hexagonal architecture and crate structure."
---

oidc-exchange separates business logic from infrastructure through interfaces called ports. Adapters implement these interfaces for databases, signing keys, audit systems, and identity providers. This pattern is called hexagonal architecture. The core crate defines the ports and has no infrastructure dependencies. Configuration selects the adapters at startup.

## Crate structure

The project is a Cargo workspace with six crates, which are Rust packages:

```
crates/
├── core/           # Domain types, port traits, service logic (zero infra deps)
├── adapters/       # DynamoDB, KMS, SQS, OIDC, webhook implementations
├── providers/      # Non-standard provider modules (Apple; atproto planned)
├── server/         # Axum routes, middleware, telemetry, bootstrap
├── ffi/            # Language-agnostic request/response FFI wrapper
└── test-utils/     # Mock implementations for all ports
```

| Crate | Package name | Purpose |
|---|---|---|
| `crates/core` | `oidc-exchange-core` | Domain types, port traits, and service logic. No infrastructure crates (no AWS SDKs, HTTP clients, or database drivers). |
| `crates/adapters` | `oidc-exchange-adapters` | Implementations of port traits for DynamoDB, PostgreSQL, SQLite, Valkey, LMDB, KMS, SQS, standard OIDC, and webhooks. |
| `crates/providers` | `oidc-exchange-providers` | Non-standard identity provider modules (Apple; atproto planned) that need custom logic beyond the generic OIDC adapter. |
| `crates/server` | `oidc-exchange` | HTTP layer (axum), middleware, telemetry setup, configuration loading, and the binary entrypoint. |
| `crates/ffi` | `oidc-exchange-ffi` | Language-agnostic request/response wrapper over the server's router construction. Reused by the Node.js and Python bindings via FFI. |
| `crates/test-utils` | `oidc-exchange-test-utils` | In-memory mock implementations of all ports. Dev-dependency only. |

## Hexagonal architecture

The core depends on port interfaces. The server provides their adapter implementations at startup.

```
                    ┌─────────────────────────────────┐
                    │          server crate            │
                    │   (axum routes, middleware,       │
                    │    telemetry, bootstrap)          │
                    └──────────────┬───────────────────┘
                                   │
                    ┌──────────────▼───────────────────┐
                    │          core crate               │
                    │                                   │
                    │   AppService (orchestrator)       │
                    │   Domain types (User, Session)    │
                    │   Port traits (interfaces)        │
                    │                                   │
                    └──┬────┬────┬────┬────┬───────────┘
                       │    │    │    │    │
              ┌────────┘    │    │    │    └────────┐
              ▼             ▼    ▼    ▼             ▼
         Repository    KeyManager  AuditLog   IdentityProvider  UserSync
         (adapters)    (adapters)  (adapters)  (adapters +      (adapters)
                                               providers)
```

These boundaries support the following changes:

- Test business logic in `core` with mock adapters.
- Select DynamoDB or PostgreSQL through configuration.
- Add storage backends by implementing a trait.
- Keep network calls, data conversion, and retries in adapter crates.

## Port traits

Ports are asynchronous Rust traits defined in `crates/core/src/ports/`. They specify the operations available to the core.

| Port | Trait | Purpose | Adapters |
|---|---|---|---|
| User storage | `UserRepository` | Create, read, update, and delete user records | DynamoDB, PostgreSQL, SQLite |
| Session-only storage | `SessionRepository` | Optional override for session operations only | Valkey, LMDB |
| Key management | `KeyManager` | Sign JSON Web Tokens (JWTs) and export public keys | Local (Ed25519), AWS KMS (RSA, ECDSA), Noop |
| Audit logging | `AuditLog` | Compliance and security event recording | Noop, Stdout/Stderr, SQS |
| Identity provider | `IdentityProvider` | Code exchange, ID token validation, revocation | Standard OIDC, Apple, atproto (planned) |
| User sync | `UserSync` | Notify external systems of user lifecycle events | Webhook, Noop |
| Rate limiting | `RateLimiter` | Request and failed-authentication budgets | In-process fixed window, Noop |

All ports return `Result<T>` with a core error type. Adapters convert their internal errors to this type before returning them. The core does not depend on adapter-specific error types.

## Dependency rules

The dependency graph enforces strict layering:

- **`core`** depends on nothing infrastructure-specific: no AWS SDKs, no HTTP clients, no database drivers. Its dependencies are pure-Rust utility crates (serde, thiserror, async-trait, chrono, tracing, and similar), never infrastructure clients.
- **`adapters`** and **`providers`** depend on `core` for trait definitions. They implement the port traits using real infrastructure clients.
- **`server`** depends on `core`, `adapters`, and `providers`. It creates and connects the adapters at startup.
- **`test-utils`** depends only on `core`. It provides in-memory mock implementations used as dev-dependencies by all other crates.

The crate dependencies preserve the separation between core logic and infrastructure.

```
server ──────► core ◄────── adapters
                ▲
                │
            providers
                ▲
                │
           test-utils (dev only)
```

## AppService

The core crate's `AppService` struct holds the ports and implements token exchange, refresh, revocation, and user management:

```rust
pub struct AppService {
    user_repo: Box<dyn UserRepository>,
    session_repo: Box<dyn SessionRepository>,
    keys: Box<dyn KeyManager>,
    audit: Box<dyn AuditLog>,
    user_sync: Box<dyn UserSync>,
    rate_limiter: Box<dyn RateLimiter>,
    providers: HashMap<String, Box<dyn IdentityProvider>>,
    config: Config,
}
```

All ports use dynamic dispatch (`Box<dyn Trait>`), which selects the implementation at runtime. This lets configuration choose adapters without requiring a separate build.

At startup, the server reads the configuration and creates the adapters and `AppService`. The axum router shares an `Arc<AppService>` with request handlers.

## Runtime bootstrap

The server crate's `main.rs` is the entrypoint: it loads configuration, initializes telemetry, and selects the transport, then delegates adapter wiring and router construction to `bootstrap.rs` (`build_service` and `build_router`). The full startup sequence is:

1. Load configuration (TOML file + environment variable overrides).
2. Initialize the telemetry subscriber based on `[telemetry]` config.
3. Detect runtime mode: if `AWS_LAMBDA_RUNTIME_API` is set, Lambda mode; otherwise, server mode.
4. Instantiate adapters based on config in `bootstrap::build_service` (user repository, session repository, key manager, audit, rate limiter, providers, user sync).
5. Construct `AppService` with injected ports, also in `bootstrap::build_service`.
6. Build the axum `Router` with routes and middleware in `bootstrap::build_router`.
7. Lambda mode: wrap the router with `lambda_http` and run `lambda_runtime`. Server mode: bind to the configured address and run with hyper.

Both modes use the same binary, router, and request handlers. They differ only in how requests reach the router.
