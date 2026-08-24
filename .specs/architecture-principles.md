# Architecture Principles

**Status:** Implemented · **Date:** 2026-08-16 · **Owner:** Ant Stanley · **Scope:** Repo-wide

Repo-wide architecture for `oidc-exchange`: a Rust service that validates ID tokens from
third-party OIDC providers and exchanges them for self-issued access and refresh tokens.
This page defines the layering, the monorepo layout, the dependency rules, and the stack
baseline that every package follows. Per-package specs reference this page rather than
restating it.

## What the system is

One Rust core, surfaced four ways:

1. A **standalone binary** (`oidc-exchange`) that runs as an axum HTTP server or — when
   `AWS_LAMBDA_RUNTIME_API` is set — an AWS Lambda function.
2. An **in-process library** for Node.js (`@oidc-exchange/node`) and Python
   (`oidc-exchange`) via FFI bindings over a shared `crates/ffi` layer.
3. An **AWS Lambda adapter** (`@oidc-exchange/lambda`) that wraps the Node binding.
4. A set of **satellite apps** — a SvelteKit admin UI and an Astro documentation site —
   that consume the service over its HTTP API.

The same domain logic and the same axum router back all of these. Only the outermost
transport differs.

## Hexagonal architecture

The Rust workspace is a strict ports-and-adapters (hexagonal) design.

- **Core** (`crates/core`) holds the domain model, the port traits, and the service
  orchestration. It depends only on `serde`, `thiserror`, `async-trait`, `chrono`,
  `tracing`, `ulid`, and crypto/encoding helpers. It has **zero** knowledge of AWS, HTTP,
  SQL, or any concrete infrastructure.
- **Adapters** (`crates/adapters`) and **providers** (`crates/providers`) implement the
  core's port traits against real infrastructure (DynamoDB, KMS, Postgres, OIDC servers,
  Apple, …). They depend on `core`; `core` never depends on them.
- **Server** (`crates/server`) is the HTTP layer. It wires adapters to ports at startup
  based on configuration, builds the axum router, and runs it as a server or Lambda.
- **FFI** (`crates/ffi`) re-uses the server's router-building logic and exposes a small,
  synchronous, language-agnostic request/response interface for the bindings.

The rule is one-directional: dependencies point **inward** toward `core`. An adapter maps
its native errors (an `SdkError`, an `sqlx::Error`) into the core's [`Error`](service/specs/00-overview.md)
type at the boundary, so the core never sees an infrastructure type.

```
                    ┌─────────────────────────────────────────┐
   transports  ──►  │  server (axum)   ffi   bindings   apps   │
                    └───────────────┬─────────────────────────┘
                                    │ implements / calls
                    ┌───────────────▼─────────────────────────┐
   adapters   ──►   │ dynamo postgres sqlite lmdb valkey       │
   providers        │ kms local-keys oidc apple webhook        │
                    │ stdout-audit sqs-audit noop              │
                    └───────────────┬─────────────────────────┘
                                    │ implement port traits
                    ┌───────────────▼─────────────────────────┐
   core       ──►   │  domain   ports   service   config       │  (no infra deps)
                    └──────────────────────────────────────────┘
```

## Why dynamic dispatch

Every port is IO-bound (a network call, a disk write, a KMS sign). The service holds its
ports as `Box<dyn Trait>` trait objects so the concrete adapter is chosen at runtime from
configuration. The nanosecond cost of virtual dispatch is irrelevant next to the IO it
guards, and runtime selection is what lets one binary serve every deployment shape.

## Fail closed

A security control that cannot be evaluated denies. A configuration that cannot be
validated refuses to start. Neither degrades to the permissive interpretation, and neither
defers the decision to the first request that depends on it.

Three rules follow, and every crate observes them:

1. **Closed value domains.** A configuration field that selects a security control is a
typed enum or newtype whose constructor is the only way to obtain a value. Comparing an
operator-supplied `String` by equality against one literal is the anti-pattern this
replaces: it makes the unrecognised case indistinguishable from the deliberate one, and it
always resolves to whichever branch the `==` did not select.
2. **Reject at startup, not at request time.** Wherever the input is configuration, the
rejection belongs in config load. A service that will never work correctly refuses to boot
rather than running in a weakened mode and reporting itself healthy. Request paths consume
already-narrowed types and have no fallback branch to take.
3. **A control that could not run did not pass.** An unread HTTP status, an absent hashing
utility, or a probe that answers a weaker question than the invariant it stands in for are
failures, not silence. Where a degraded path is genuinely wanted (a DDL-denied database role
or an out-of-band migration), it is reached by explicit configuration and still verifies the
invariant it is skipping enforcement of.

The service loads configuration once, at startup; there is no reload path, so these
guarantees are established exactly once per process.

## Monorepo layout

```
oidc-exchange/
├── Cargo.toml                 # Rust workspace root
├── pnpm-workspace.yaml        # JS/TS workspace (bindings/nodejs, bindings/lambda, apps/*)
├── config/default.toml        # compiled-in default configuration
├── schemas/                   # JSON Schema data model + DynamoDB table design
├── crates/
│   ├── core/                  # domain + ports + service (zero infra deps)
│   ├── adapters/              # port implementations
│   ├── providers/             # non-standard identity providers (Apple)
│   ├── server/                # axum HTTP layer + bootstrap + telemetry
│   ├── ffi/                   # language-agnostic request/response wrapper
│   └── test-utils/            # mock ports (dev-dependency)
├── bindings/
│   ├── nodejs/                # napi-rs → @oidc-exchange/node
│   ├── python/                # PyO3 + maturin → oidc-exchange (PyPI)
│   └── lambda/                # TypeScript → @oidc-exchange/lambda
├── apps/
│   ├── admin-ui/              # SvelteKit admin console
│   └── website/              # Astro/Starlight docs site
├── examples/                  # reference deployments and framework integrations
└── docs/                      # canonical prose docs (website symlinks this)
```

The Cargo workspace members are `crates/{core,adapters,providers,server,ffi,test-utils}`
plus `bindings/{nodejs,python}` (both are `cdylib` crates). The pnpm workspace covers
`bindings/nodejs`, `bindings/lambda`, and `apps/*`.

## Dependency graph (Rust)

```
core ◄── adapters ◄── server ◄── ffi ◄── bindings/nodejs ◄── bindings/lambda
  ▲          ▲           ▲          ▲
  └── providers ─────────┘          └── bindings/python
test-utils ─► core           (dev-dependency of all crates)
```

- `core` depends on nothing in the workspace.
- `adapters` and `providers` depend on `core`.
- `server` depends on `core`, `adapters`, `providers`.
- `ffi` depends on `server` (router construction) and `core`.
- Node and Python bindings depend on `ffi`. The Lambda binding depends on the Node binding.

`providers` is a separate crate from `adapters` because providers are user-facing identity
integrations that can carry heavy, protocol-specific logic (Apple's per-request ES256
client JWT), whereas adapters are infrastructure backends.

## Runtime modes

The binary detects its mode at startup:

- **Server mode** (default) — binds `server.host:server.port` and serves over hyper.
- **Lambda mode** — selected when `AWS_LAMBDA_RUNTIME_API` is present.

A `server.role` setting (`all` | `exchange` | `admin`) selects which route groups mount and
which adapters are constructed, so the same binary can run as a public exchange service, an
internal admin service, or both. See [service/specs/04-http-api.md](service/specs/04-http-api.md).

## Stack baseline

| Concern | Choice |
|---|---|
| Core language | Rust (edition 2021) |
| HTTP framework | axum (tower-based, Lambda-compatible) |
| Async runtime | tokio |
| Config format | TOML |
| Serialization | serde / serde_json |
| Error modelling | thiserror, domain `Error` enum |
| IDs | ULID, prefixed (`usr_…`) |
| Node bindings | napi-rs |
| Python bindings | PyO3 + maturin (abi3, Python 3.10+) |
| Admin UI | SvelteKit + adapter-node + Tailwind 4 + LayerChart |
| Docs site | Astro + Starlight |
| Test runner (Rust) | cargo-nextest |
| Version control | Jujutsu (jj) over a Git backend |

Toolchain commands, code style, and the definition of done live in
[development-guidelines.md](development-guidelines.md).

## Assumptions and open questions

### Assumptions

- The core compiles without any cloud SDK present; AWS SDKs live only in `crates/adapters`.
- All transports share one axum router, so HTTP semantics are identical across server,
  Lambda, and FFI bindings.

### Decisions

- *Hexagonal core.* **Domain logic is isolated in `crates/core` with no infra deps.** Lets
  adapters be swapped (DynamoDB → Postgres, KMS → local keys) by implementing a trait, and
  lets the core be unit-tested with in-memory mocks from `crates/test-utils`.
- *Trait objects over generics.* **Ports are `Box<dyn Trait>`.** All ports are IO-bound, so
  runtime adapter selection from config outweighs dispatch cost.
- *Split repositories.* **`UserRepository` and `SessionRepository` are separate traits.**
  Sessions and users have different storage characteristics; this lets a deployment back
  sessions with LMDB or Valkey while keeping users in SQL or DynamoDB.
- *Single binary, runtime selection.* **One binary serves server and Lambda, every adapter
  compiled in.** Simplifies distribution: install once, configure with TOML.
- *FFI re-uses the server router.* **Bindings call the same axum router via `crates/ffi`.**
  Avoids a second HTTP implementation and keeps behaviour identical in-process and over the
  wire.
- *Fail closed.* **A security control that cannot be evaluated denies; a configuration that
  cannot be validated refuses to start.** Closed value domains, rejection at startup, and
  could-not-run-did-not-pass replace per-site permissive fallbacks — the three rules in the
  *Fail closed* section above.

### Open questions

- `bindings/lambda` is a pnpm workspace member but not a Cargo member (it is pure
  TypeScript); the two workspace manifests are maintained independently, with version
  parity enforced only by the release pipeline. Whether to unify version bumps is open.
