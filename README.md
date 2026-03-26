# oidc-exchange

A Rust service that validates ID tokens from third-party OIDC providers and exchanges them for self-issued access and refresh tokens. Built with hexagonal architecture for pluggable infrastructure, configurable via TOML, and deployable as a Lambda function or long-lived server from a single binary.

## Why oidc-exchange?

If your application needs to authenticate users via Google, Apple, or other OIDC providers, you typically have three choices: a hosted auth service (Auth0, Cognito, Firebase Auth), a full-blown self-hosted OIDC server (Keycloak, Dex, Ory Hydra), or rolling your own token validation. Each comes with trade-offs that oidc-exchange is designed to avoid.

### Compared to hosted auth services (Auth0, Cognito, Firebase Auth)

Hosted services are convenient but introduce external dependencies that affect cost, latency, and control:

- **No per-MAU pricing** — oidc-exchange runs on your own infrastructure. You pay for compute and storage, not per authenticated user.
- **No vendor lock-in** — your user data stays in your database, your tokens are signed with your keys, and your configuration is a TOML file in your repo.
- **No opaque behavior** — every decision (registration policy, claims mapping, token lifetime) is explicit in configuration. There are no hidden rules or console toggles to discover in production.
- **Lower latency** — token exchange happens in-process or within your VPC. There is no round-trip to a third-party service on every authentication.

### Compared to full OIDC servers (Keycloak, Dex, Ory Hydra)

Full OIDC servers are designed to be the identity provider — they manage user credentials, host login pages, and implement the full OAuth 2.0 authorization server spec. If you are delegating authentication to external providers and just need to issue your own tokens, they are dramatically over-scoped:

- **No login UI to maintain** — oidc-exchange does not host login pages or manage passwords. Your client handles the provider's OAuth flow and sends the resulting code or ID token. The service validates and exchanges.
- **No session management** — there are no server-side sessions, cookies, or consent screens. You get a JWT and a refresh token.
- **Single-purpose** — the entire codebase does one thing: validate upstream identity, issue downstream tokens. This makes it auditable, testable, and operationally simple.
- **Minutes to deploy, not days** — a single binary, a TOML config, and a DynamoDB table. No database migrations, no admin consoles, no clustering configuration.

### Compared to rolling your own

Writing token validation and JWT issuance from scratch is straightforward until it isn't:

- **Provider quirks handled** — Apple requires generating a per-request ES256 client JWT instead of using a static client secret. Standard OIDC libraries don't account for this. oidc-exchange does.
- **Security defaults** — refresh tokens are stored hashed (SHA-256), access tokens are short-lived, registration policy enforcement and domain allowlists are built in.
- **Audit trail included** — every token exchange, revocation, and user event can be logged to CloudTrail Lake with syslog severity levels. Adding this after the fact is painful.
- **Hexagonal architecture** — swapping DynamoDB for Postgres or KMS for Vault means implementing a trait, not rewriting the service.

### When to use something else

oidc-exchange is not a general-purpose authorization server. Choose a different tool if you need:

- **Password-based authentication** — oidc-exchange delegates authentication entirely to upstream providers.
- **OAuth 2.0 authorization server** — if you need to issue tokens to third-party clients with scopes and consent, use a full OIDC server.
- **Multi-tenant SaaS auth** — if you need organization management, RBAC, or SCIM provisioning, a hosted service like Auth0 or WorkOS is better suited.
- **Federation between internal services** — if you need service-to-service authentication (mTLS, SPIFFE), oidc-exchange is the wrong layer.

## Features

- **Token Exchange** — accepts authorization codes from OIDC providers, validates ID tokens, issues short-lived JWTs (default 15min) and long-lived refresh tokens (default 30 days)
- **Pluggable Providers** — three tiers: standard OIDC (Google, config-only), OIDC-with-quirks (Apple, ES256 client JWT), and non-OIDC (atproto, planned)
- **Hexagonal Architecture** — all infrastructure behind trait interfaces: database, key management, audit, user sync
- **Registration Policy** — open or existing-users-only mode with optional email domain/subdomain allowlists
- **Per-User Claims** — configurable custom JWT claims from TOML templates and per-user overrides via internal API
- **Audit Trail** — syslog severity levels, configurable blocking threshold, CloudTrail Lake integration, stdout/stderr fallback
- **OpenTelemetry** — pluggable exporters (OTLP, X-Ray, stdout) via `tracing` ecosystem
- **Dual Runtime** — same binary runs as an axum server or AWS Lambda function
- **Internal Admin API** — user CRUD and claims management with shared-secret authentication

## Architecture

```
crates/
├── core/           # Domain types, port traits, service logic (zero infra deps)
├── adapters/       # DynamoDB, KMS, CloudTrail, OIDC, webhook implementations
├── providers/      # Non-standard provider modules (Apple, atproto)
├── server/         # Axum routes, middleware, telemetry, bootstrap
└── test-utils/     # Mock implementations for all ports
```

### Ports (Trait Interfaces)

| Port | Purpose | Adapters |
|------|---------|----------|
| `Repository` | User and session storage | DynamoDB |
| `KeyManager` | JWT signing | Local (Ed25519), AWS KMS |
| `AuditLog` | Compliance event logging | CloudTrail Lake, Noop |
| `IdentityProvider` | OIDC provider interaction | Standard OIDC, Apple |
| `UserSync` | Bidirectional user sync | Webhook, Noop |

## Quick Start

### Prerequisites

- Rust 1.75+
- [cargo-nextest](https://nexte.st) for testing

### Build

```bash
cargo build --release
```

### Configure

Create a `config.toml` (or set `OIDC_EXCHANGE_ENV` to load `config/{env}.toml`):

```toml
[server]
host = "0.0.0.0"
port = 8080
issuer = "https://auth.example.com"

[registration]
mode = "open"
# domain_allowlist = ["example.com", "*.acme.corp"]

[token]
access_token_ttl = "15m"
refresh_token_ttl = "30d"
audience = "https://api.example.com"

[token.custom_claims]
org = "example"
role = "{{ user.metadata.role | default: 'user' }}"

[key_manager]
adapter = "local"

[key_manager.local]
private_key_path = "./keys/ed25519.pem"
algorithm = "EdDSA"
kid = "key-1"

[repository]
adapter = "dynamodb"

[repository.dynamodb]
table_name = "oidc-exchange"

[audit]
adapter = "noop"
blocking_threshold = "warning"

[providers.google]
adapter = "oidc"
issuer = "https://accounts.google.com"
client_id = "${GOOGLE_CLIENT_ID}"
client_secret = "${GOOGLE_CLIENT_SECRET}"
scopes = ["openid", "email", "profile"]
```

### Run

```bash
./target/release/oidc-exchange
```

The server starts on the configured host/port. Set `AWS_LAMBDA_RUNTIME_API` to run in Lambda mode.

## API Endpoints

### Public

| Method | Path | Description |
|--------|------|-------------|
| POST | `/token` | Token exchange (`grant_type=authorization_code`) and refresh (`grant_type=refresh_token`) |
| POST | `/revoke` | Token revocation (RFC 7009) |
| GET | `/keys` | JWKS endpoint |
| GET | `/.well-known/openid-configuration` | OpenID Connect discovery |
| GET | `/health` | Health check |

### Internal (requires `Authorization: Bearer <secret>`)

| Method | Path | Description |
|--------|------|-------------|
| POST | `/internal/users` | Create user |
| GET | `/internal/users/{id}` | Get user |
| PATCH | `/internal/users/{id}` | Update user |
| DELETE | `/internal/users/{id}` | Soft-delete user |
| GET | `/internal/users/{id}/claims` | Get user claims |
| PUT | `/internal/users/{id}/claims` | Replace user claims |
| PATCH | `/internal/users/{id}/claims` | Merge user claims |
| DELETE | `/internal/users/{id}/claims` | Clear user claims |

## Token Exchange Flow

```
Client → Authorization Code + Provider → POST /token
  → Provider validates code, returns ID token
  → Service validates ID token (signature, iss, aud, exp)
  → Registration policy check (domain allowlist, mode)
  → User lookup/creation
  → Generate refresh token (256-bit random, stored hashed)
  → Sign access token JWT (short-lived)
  → Return { access_token, refresh_token, token_type, expires_in }
```

## Configuration

Config is loaded in order:
1. `config/default.toml`
2. `config/{OIDC_EXCHANGE_ENV}.toml` (if set)
3. Environment variable overrides: `OIDC_EXCHANGE__{section}__{key}`
4. `${VAR_NAME}` placeholder resolution from environment

See `config/default.toml` for the minimal default configuration.

## Testing

```bash
# Run all tests
cargo nextest run --workspace

# Run only core logic tests
cargo nextest run -p oidc-exchange-core

# Run adapter tests (some require Docker)
cargo nextest run -p oidc-exchange-adapters

# Run server/E2E tests
cargo nextest run -p oidc-exchange

# Run DynamoDB integration tests (requires DynamoDB Local)
docker run -p 8000:8000 amazon/dynamodb-local
cargo nextest run -p oidc-exchange-adapters -- --ignored
```

## Deployment Guides

See [docs/integration/](docs/integration/README.md) for detailed deployment guides:

| Guide | Best for |
|-------|----------|
| [AWS Lambda](docs/integration/aws-lambda.md) | Serverless, pay-per-request |
| [ECS Fargate](docs/integration/ecs-fargate.md) | Auto-scaling containers with ALB |
| [Linux + PostgreSQL](docs/integration/linux-postgres.md) | Relational storage, optional Valkey |
| [Linux + SQLite](docs/integration/linux-sqlite.md) | Single-server, zero dependencies |
| [Generic Container](docs/integration/container.md) | K8s, Cloud Run, any orchestrator |
| [Generic Linux](docs/integration/linux-server.md) | On-prem, simple single-server |

## Project Structure

```
oidc-exchange/
├── Cargo.toml                    # Workspace root
├── .config/nextest.toml          # Test runner config
├── config/default.toml           # Default configuration
├── schemas/
│   ├── datamodel.schema.json     # Generic domain model (adapter-agnostic)
│   └── dynamodb/table-design.json # DynamoDB single-table design
├── crates/
│   ├── core/                     # Domain + ports + service logic
│   ├── adapters/                 # Infrastructure implementations
│   ├── providers/                # Non-standard OIDC providers
│   ├── server/                   # HTTP layer + bootstrap
│   └── test-utils/               # Mock implementations
└── docs/
    ├── integration/              # Deployment guides
    ├── contributing.md           # Contributing guide
    └── superpowers/
        ├── specs/                # Design specification
        └── plans/                # Implementation plan
```

## License

MIT
