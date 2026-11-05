---
title: Deployment Overview
description: Choose a deployment model for oidc-exchange.
---

Choose a deployment guide for your infrastructure. All deployment targets use the same binary. Environment variables and configuration determine how it runs.

## Choosing a deployment model

| Guide | Best for | Storage |
|-------|----------|---------|
| [AWS Lambda](/deployment/aws-lambda/) | Serverless, pay-per-request, AWS-native | DynamoDB |
| [ECS Fargate](/deployment/ecs-fargate/) | Containerized, auto-scaling, high availability | DynamoDB + ElastiCache Valkey |
| [Linux + PostgreSQL](/deployment/linux-postgres/) | Traditional server, relational storage | PostgreSQL (+ optional Valkey) |
| [Linux + SQLite](/deployment/linux-sqlite/) | Single-server, minimal dependencies | SQLite (+ optional LMDB) |
| [Generic Container](/deployment/container/) | Kubernetes, Cloud Run, or another container platform | Any supported backend |
| [Generic Linux](/deployment/linux-server/) | A single server on your own infrastructure | Any supported backend |

## Prerequisites

All environments require:

- A built `oidc-exchange` binary (see [Building](#building)).
- A configuration file in TOML format (see [Configuration](#configuration)).
- At least one configured OpenID Connect (OIDC) identity provider, such as Google or Apple.

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

1. `config/default.toml` (baseline defaults).
2. `config/{OIDC_EXCHANGE_ENV}.toml` (environment-specific overrides).
3. Environment variables, `OIDC_EXCHANGE__{section}__{key}` (double underscore delimiters).
4. `${VAR_NAME}` placeholders, resolved from environment at load time.

Use `${VAR_NAME}` placeholders for secrets such as client secrets and API keys. Supply their values through environment variables. Do not store secret values in TOML files.

## Client integration

Clients use the same endpoints for every deployment method.

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

Your API verifies access tokens with public keys from the JSON Web Key Set (JWKS) endpoint:

```bash
curl https://auth.example.com/keys
```

Use a JSON Web Token (JWT) library with JWKS support. Configure it to read `https://auth.example.com/keys` and cache and refresh keys according to the library's documentation.

### OpenID Connect discovery

```bash
curl https://auth.example.com/.well-known/openid-configuration
```

This returns the standard discovery document, including the JWKS URI, supported grant types, and token endpoint.
