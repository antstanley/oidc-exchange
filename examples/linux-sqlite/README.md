---
title: "Linux + SQLite Example"
description: "Simplest oidc-exchange deployment using SQLite for storage, with an optional LMDB session store."
version: "0.1"
last_updated: 2026-03-26
---

# Linux + SQLite Example

> **Tip:** Prebuilt Docker images are available at `ghcr.io/antstanley/oidc-exchange:latest`. See the [Docker guide](https://github.com/antstanley/oidc-exchange/blob/main/apps/website/src/content/docs/guides/docker.md) for details.

The simplest deployment of oidc-exchange: no external services, everything on disk.
Two modes are provided:

- **SQLite-only** -- SQLite handles both the user/token repository and sessions.
- **SQLite + LMDB** -- SQLite for the user/token repository, LMDB for sessions (faster session lookups under load).

## Prerequisites

- **cargo** -- to build oidc-exchange
- **openssl** -- for signing-key generation

## Quick start

1. Build the binary:

   ```bash
   cargo build --release
   ```

2. Run the setup script (creates directories, generates a signing key, copies config files):

   ```bash
   chmod +x ./examples/linux-sqlite/setup.sh
   ./examples/linux-sqlite/setup.sh
   ```

3. Set environment variables and start the server:

   ```bash
   OIDC_EXCHANGE_ENV=sqlite-only \
     GOOGLE_CLIENT_ID=your-client-id \
     GOOGLE_CLIENT_SECRET=your-client-secret \
     ./target/release/oidc-exchange
   ```

4. Verify it is running:

   ```bash
   curl http://localhost:8080/health
   ```

## LMDB variant

Follow the same setup steps above, then start with the `sqlite-lmdb` config instead:

```bash
OIDC_EXCHANGE_ENV=sqlite-lmdb \
  GOOGLE_CLIENT_ID=your-client-id \
  GOOGLE_CLIENT_SECRET=your-client-secret \
  ./target/release/oidc-exchange
```

## Querying the database

```bash
sqlite3 data/oidc-exchange.db "SELECT id, email, status FROM users;"
```

## Cleanup

Remove all generated data:

```bash
rm -rf data/ lmdb/ keys/
```

## Further reading

See [docs/integration/linux-sqlite.md](../../docs/integration/linux-sqlite.md) for a deeper walkthrough.
