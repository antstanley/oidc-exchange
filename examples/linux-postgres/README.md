---
title: "Linux + PostgreSQL Example"
description: "Run oidc-exchange on Linux with PostgreSQL for persistence, optionally adding Valkey for session storage."
version: "0.1"
last_updated: 2026-03-26
---

# Linux + PostgreSQL Example

> **Tip:** Prebuilt Docker images are available at `ghcr.io/antstanley/oidc-exchange:latest`. See the [Docker guide](https://github.com/antstanley/oidc-exchange/blob/main/apps/website/src/content/docs/guides/docker.md) for details.

This example shows how to run **oidc-exchange** on a Linux host with PostgreSQL as the backing store. Two deployment modes are provided:

| Mode | Config file | What it runs |
|------|-------------|--------------|
| **PostgreSQL-only** | `config/postgres-only.toml` | PostgreSQL handles both users and sessions. |
| **PostgreSQL + Valkey** | `config/postgres-valkey.toml` | PostgreSQL for users, Valkey (Redis-compatible) for sessions. |

## Prerequisites

- **Docker** and **Docker Compose** (to run the database containers)
- **cargo** (to build oidc-exchange from source)
- **openssl** (to generate the signing key)

## Quick start (PostgreSQL-only)

1. **Build the binary**

   ```sh
   cargo build --release
   ```

2. **Start PostgreSQL**

   ```sh
   docker compose up -d
   ```

3. **Generate a signing key**

   ```sh
   mkdir -p keys
   openssl genpkey -algorithm ed25519 -out keys/signing-key.pem
   ```

4. **Set environment variables and run**

   ```sh
   OIDC_EXCHANGE_ENV=postgres-only \
   GOOGLE_CLIENT_ID=your-client-id \
   GOOGLE_CLIENT_SECRET=your-client-secret \
     ./target/release/oidc-exchange
   ```

5. **Verify**

   ```sh
   curl http://localhost:8080/health
   ```

## With Valkey (PostgreSQL + Valkey)

1. **Start both PostgreSQL and Valkey**

   ```sh
   docker compose -f docker-compose.yml -f docker-compose.valkey.yml up -d
   ```

2. **Run with the Valkey-enabled config**

   ```sh
   OIDC_EXCHANGE_ENV=postgres-valkey \
   GOOGLE_CLIENT_ID=your-client-id \
   GOOGLE_CLIENT_SECRET=your-client-secret \
     ./target/release/oidc-exchange
   ```

## Further reading

See [docs/integration/linux-postgres.md](../../docs/integration/linux-postgres.md) for a full integration guide.
