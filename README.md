# oidc-exchange

A Rust service that validates ID tokens from third-party OIDC providers and exchanges them for self-issued access and refresh tokens. Built with hexagonal architecture for pluggable infrastructure, configurable via TOML, and deployable as a Lambda function or long-lived server from a single binary.

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
    └── superpowers/
        ├── specs/                # Design specification
        └── plans/                # Implementation plan
```

## License

MIT
