---
title: "Architecture"
description: "Hexagonal architecture and crate structure."
---

oidc-exchange is built with hexagonal architecture (ports and adapters). All infrastructure --- databases, key management, audit systems, identity providers --- sits behind trait interfaces defined in the core crate. The core business logic has zero infrastructure dependencies. Adapters are selected at runtime from configuration.

## Crate structure

The project is a Cargo workspace with five crates:

```
crates/
├── core/           # Domain types, port traits, service logic (zero infra deps)
├── adapters/       # DynamoDB, KMS, CloudTrail, OIDC, webhook implementations
├── providers/      # Non-standard provider modules (Apple, atproto)
├── server/         # Axum routes, middleware, telemetry, bootstrap
└── test-utils/     # Mock implementations for all ports
```

| Crate | Package name | Purpose |
|---|---|---|
| `crates/core` | `oidc-exchange-core` | Domain types, port traits, and service logic. Zero infrastructure dependencies --- only std, serde, thiserror, async-trait, and tracing. |
| `crates/adapters` | `oidc-exchange-adapters` | Implementations of port traits for DynamoDB, PostgreSQL, SQLite, Valkey, LMDB, KMS, CloudTrail, SQS, standard OIDC, and webhooks. |
| `crates/providers` | `oidc-exchange-providers` | Non-standard identity provider modules (Apple, atproto) that need custom logic beyond the generic OIDC adapter. |
| `crates/server` | `oidc-exchange` | HTTP layer (axum), middleware, telemetry setup, configuration loading, and the binary entrypoint. |
| `crates/test-utils` | `oidc-exchange-test-utils` | In-memory mock implementations of all ports. Dev-dependency only. |

## Hexagonal architecture

The hexagonal architecture pattern separates business logic from infrastructure by defining abstract interfaces (ports) that the core depends on. Concrete implementations (adapters) are injected at startup.

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

This means:

- Business logic in `core` is testable in complete isolation using mocks
- Swapping DynamoDB for PostgreSQL means changing a config value, not rewriting the service
- Adding a new storage backend means implementing a trait, not modifying core logic
- All infrastructure concerns (network, serialization, retries) are contained in adapter crates

## Port traits

Ports are async trait interfaces defined in `crates/core/src/ports/`. They define the contracts that the core depends on.

| Port | Trait | Purpose | Adapters |
|---|---|---|---|
| User and session storage | `Repository` | CRUD for users, store/retrieve/revoke refresh token sessions | DynamoDB, PostgreSQL, SQLite |
| Session-only storage | `SessionRepository` | Optional override for session operations only | Valkey, LMDB |
| Key management | `KeyManager` | JWT signing and public key export | Local (Ed25519/ECDSA), AWS KMS (ECC_NIST_P256) |
| Audit logging | `AuditLog` | Compliance and security event recording | Noop, CloudTrail Lake, SQS |
| Identity provider | `IdentityProvider` | Code exchange, ID token validation, revocation | Standard OIDC, Apple, atproto |
| User sync | `UserSync` | Notify external systems of user lifecycle events | Webhook, Noop |

All ports return `Result<T>` using a domain-specific error type. Adapters map their internal errors (AWS SDK errors, database errors, HTTP errors) into domain errors at the boundary. No adapter-specific types leak into the core.

## Dependency rules

The dependency graph enforces strict layering:

- **`core`** depends on nothing infrastructure-specific. No AWS SDKs, no HTTP clients, no database drivers. Only std + serde + async-trait + thiserror + tracing.
- **`adapters`** and **`providers`** depend on `core` for trait definitions. They implement the port traits using real infrastructure clients.
- **`server`** depends on `core`, `adapters`, and `providers`. It wires everything together at startup.
- **`test-utils`** depends only on `core`. It provides in-memory mock implementations used as dev-dependencies by all other crates.

These boundaries are enforced by the Cargo workspace. If `core` compiles, the domain logic is free of infrastructure coupling.

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

The `AppService` struct in the core crate is the central orchestrator. It holds references to all ports and implements the business logic for token exchange, refresh, revocation, and user management:

```rust
pub struct AppService {
    repo: Box<dyn Repository>,
    keys: Box<dyn KeyManager>,
    audit: Box<dyn AuditLog>,
    user_sync: Box<dyn UserSync>,
    providers: HashMap<String, Box<dyn IdentityProvider>>,
    config: AppConfig,
}
```

All ports use dynamic dispatch (`Box<dyn Trait>`). Since every port operation is I/O-bound (network calls, disk reads), the nanosecond overhead of dynamic dispatch is irrelevant compared to the millisecond cost of the actual operations. This enables runtime adapter selection from configuration without monomorphization complexity.

The server crate constructs `AppService` at startup by reading the configuration, instantiating the appropriate adapters, and injecting them. The axum router holds an `Arc<AppService>` in application state and passes it to request handlers.

## Runtime bootstrap

The `main.rs` in the server crate follows this sequence:

1. Load configuration (TOML file + environment variable overrides)
2. Initialize the telemetry subscriber based on `[telemetry]` config
3. Detect runtime mode: if `AWS_LAMBDA_RUNTIME_API` is set, Lambda mode; otherwise, server mode
4. Instantiate adapters based on config (repository, key manager, audit, providers, user sync)
5. Construct `AppService` with injected ports
6. Build the axum `Router` with routes and middleware
7. Lambda mode: wrap the router with `lambda_http` and run `lambda_runtime`. Server mode: bind to the configured address and run with hyper.

The same binary, the same router, and the same code paths run in both modes. Only the outermost transport layer differs.
