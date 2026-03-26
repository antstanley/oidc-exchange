---
title: Integration Guides
description: Deployment guides for oidc-exchange across AWS serverless, container, and Linux server environments.
version: "0.2"
last_updated: 2026-03-26
---

# Integration Guides

Deployment guides for oidc-exchange across different infrastructure targets. All guides use the same binary — the deployment target is determined by runtime detection and configuration.

## Choosing a deployment model

| Guide | Best for | Storage |
|-------|----------|---------|
| [AWS Lambda](aws-lambda.md) | Serverless, pay-per-request, AWS-native | DynamoDB |
| [ECS Fargate](ecs-fargate.md) | Containerized, auto-scaling, high availability | DynamoDB + ElastiCache Valkey |
| [Linux + PostgreSQL](linux-postgres.md) | Traditional server, relational storage | PostgreSQL (+ optional Valkey) |
| [Linux + SQLite](linux-sqlite.md) | Single-server, minimal dependencies | SQLite (+ optional LMDB) |
| [Generic Container](container.md) | K8s, Cloud Run, any orchestrator | Any supported backend |
| [Generic Linux](linux-server.md) | On-prem, simple single-server | Any supported backend |

---

## Prerequisites

All environments require:

- A built `oidc-exchange` binary (see [Building](#building))
- A TOML configuration file (see [Configuration](#configuration))
- At least one OIDC provider configured (Google, Apple, etc.)

### Building

```bash
# Standard server/container binary
cargo build --release
# Output: target/release/oidc-exchange

# AWS Lambda binary (requires cargo-lambda)
cargo lambda build --release
# Output: target/lambda/oidc-exchange/bootstrap
```

### Configuration

oidc-exchange loads configuration in order:

1. `config/default.toml` — baseline defaults
2. `config/{OIDC_EXCHANGE_ENV}.toml` — environment-specific overrides
3. Environment variables — `OIDC_EXCHANGE__{section}__{key}` (double underscore delimiters)
4. `${VAR_NAME}` placeholders — resolved from environment at load time

Secrets (client secrets, API keys) should always use `${VAR_NAME}` placeholders and be injected via environment variables, never hardcoded in TOML files.

---

## Client Integration

Regardless of deployment method, clients interact with oidc-exchange the same way.

### Token exchange

Your client application handles the OAuth flow with the identity provider (Google, Apple, etc.) and sends the resulting authorization code to oidc-exchange:

```bash
curl -X POST https://auth.example.com/token \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d "grant_type=authorization_code" \
  -d "code=AUTH_CODE_FROM_PROVIDER" \
  -d "provider=google" \
  -d "redirect_uri=https://app.example.com/callback"
```

Response:

```json
{
  "access_token": "eyJhbGciOi...",
  "refresh_token": "dGhpcyBpcyBh...",
  "token_type": "Bearer",
  "expires_in": 900
}
```

### Token refresh

```bash
curl -X POST https://auth.example.com/token \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d "grant_type=refresh_token" \
  -d "refresh_token=dGhpcyBpcyBh..."
```

### Token revocation

```bash
curl -X POST https://auth.example.com/revoke \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d "token=dGhpcyBpcyBh..."
```

### JWKS verification

Downstream services verify access tokens by fetching the public key from the JWKS endpoint:

```bash
curl https://auth.example.com/keys
```

Most JWT libraries support JWKS URLs natively. Point your verification middleware at `https://auth.example.com/keys` and it will cache and rotate keys automatically.

### OpenID Connect discovery

```bash
curl https://auth.example.com/.well-known/openid-configuration
```

This returns the standard discovery document, including the JWKS URI, supported grant types, and token endpoint.
