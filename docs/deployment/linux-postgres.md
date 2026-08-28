---
title: "Linux + PostgreSQL"
description: Deploy oidc-exchange with PostgreSQL for users and optional Valkey for sessions.
---

Run oidc-exchange on a Linux host using PostgreSQL for persistent storage. This guide covers PostgreSQL for both users and sessions, with an optional Valkey/Redis upgrade for session storage when you need lower-latency token operations.

A runnable example is in [`examples/linux-postgres/`](https://github.com/example/oidc-exchange/tree/main/examples/linux-postgres).

## When to use this

- You already run PostgreSQL and prefer a single relational database for all state
- You want ACID guarantees on user records
- You need to query user data directly via SQL tooling
- Optionally: you want sub-millisecond session lookups by adding Valkey/Redis

## Architecture

```
                    ┌──────────────┐
                    │    nginx     │
                    │  (TLS term)  │
                    └──────┬───────┘
                           │
                    ┌──────┴───────┐
                    │oidc-exchange │
                    └──┬───────┬───┘
                       │       │
              ┌────────┴──┐  ┌─┴──────────┐
              │ PostgreSQL │  │   Valkey    │
              │  (users)   │  │ (sessions)  │
              └────────────┘  └─────────────┘
                                 optional
```

Without Valkey, PostgreSQL handles both users and sessions.

## Prerequisites

- A Linux server with oidc-exchange binary (see [build instructions](/deployment/overview/#building))
- PostgreSQL 14+ accessible from the server
- (Optional) Valkey or Redis 7+ for session storage

## Step-by-step

### 1. Set up PostgreSQL

Create a database and user:

```bash
sudo -u postgres psql <<'SQL'
CREATE USER oidc_exchange WITH PASSWORD 'change-me';
CREATE DATABASE oidc_exchange OWNER oidc_exchange;
SQL
```

oidc-exchange runs its own migrations on startup, creating and updating the `users`, `sessions`, `retired_refresh_tokens`, and `single_use` tables. The authoritative schema is the adapter's `MIGRATIONS` block in `crates/adapters/src/postgres/mod.rs`, applied idempotently on every start, so you never create tables by hand. If you must pre-create the schema (for example, when the application role lacks DDL permission), copy that migration verbatim rather than an approximation. In particular the users uniqueness constraint is a partial unique index on `(external_id, provider) WHERE status != 'deleted'`, not a full unique index on `external_id` alone; the wrong shape blocks re-registration of a soft-deleted identity and collides the same `external_id` across two providers.

### 2. Generate a signing key

```bash
sudo mkdir -p /etc/oidc-exchange
openssl genpkey -algorithm ed25519 -out /etc/oidc-exchange/signing-key.pem
chmod 600 /etc/oidc-exchange/signing-key.pem
```

### 3. Configure (PostgreSQL only)

Create `/etc/oidc-exchange/config/production.toml`:

```toml
[server]
host = "127.0.0.1"
port = 8080
issuer = "https://auth.example.com"

[key_manager]
adapter = "local"

[key_manager.local]
private_key_path = "/etc/oidc-exchange/signing-key.pem"
algorithm = "EdDSA"
kid = "prod-1"

[repository]
adapter = "postgres"

[repository.postgres]
url = "${DATABASE_URL}"
max_connections = 10

[audit]
adapter = "stdout"
durability = "observe"

[telemetry]
enabled = true
exporter = "stdout"

[providers.google]
adapter = "oidc"
issuer = "https://accounts.google.com"
client_id = "${GOOGLE_CLIENT_ID}"
client_secret = "${GOOGLE_CLIENT_SECRET}"
scopes = ["openid", "email", "profile"]
# Origins Google's discovery document may name beyond the issuer's origin:
endpoint_origins = ["https://oauth2.googleapis.com", "https://www.googleapis.com"]
```

`endpoint_origins` pins which origins a provider's discovery document is allowed to name; each entry must be a bare `https://host[:port]`, and an unpinned origin logs a warning when discovered (see [Identity Providers](/guides/providers/)).

With this configuration, both users and sessions are stored in PostgreSQL.

### 4. Configure (PostgreSQL + Valkey for sessions)

To offload session storage to Valkey/Redis, add a `[session_repository]` section. Users stay in PostgreSQL; sessions move to Valkey with automatic TTL expiration:

```toml
[server]
host = "127.0.0.1"
port = 8080
issuer = "https://auth.example.com"

[key_manager]
adapter = "local"

[key_manager.local]
private_key_path = "/etc/oidc-exchange/signing-key.pem"
algorithm = "EdDSA"
kid = "prod-1"

# Users in PostgreSQL
[repository]
adapter = "postgres"

[repository.postgres]
url = "${DATABASE_URL}"
max_connections = 10

# Sessions in Valkey
[session_repository]
adapter = "valkey"

[session_repository.valkey]
url = "${VALKEY_URL}"
key_prefix = "oidc:"

[audit]
adapter = "stdout"
durability = "observe"

[telemetry]
enabled = true
exporter = "stdout"

[providers.google]
adapter = "oidc"
issuer = "https://accounts.google.com"
client_id = "${GOOGLE_CLIENT_ID}"
client_secret = "${GOOGLE_CLIENT_SECRET}"
scopes = ["openid", "email", "profile"]
```

When `[session_repository]` is omitted, sessions use the same adapter as `[repository]`. When present, it overrides only session storage.

### 5. Create the environment file

```bash
cat > /etc/oidc-exchange/env <<'EOF'
DATABASE_URL=postgres://oidc_exchange:change-me@localhost:5432/oidc_exchange
GOOGLE_CLIENT_ID=your-client-id
GOOGLE_CLIENT_SECRET=your-client-secret
EOF

# Include Valkey URL if using the split configuration
echo 'VALKEY_URL=redis://localhost:6379' >> /etc/oidc-exchange/env

chmod 600 /etc/oidc-exchange/env
```

### 6. Create the systemd service

```ini
# /etc/systemd/system/oidc-exchange.service

[Unit]
Description=oidc-exchange token service
After=network-online.target postgresql.service
Wants=network-online.target

[Service]
Type=simple
User=oidc-exchange
Group=oidc-exchange
ExecStart=/usr/local/bin/oidc-exchange
WorkingDirectory=/etc/oidc-exchange
Restart=on-failure
RestartSec=5

EnvironmentFile=/etc/oidc-exchange/env
Environment=OIDC_EXCHANGE_ENV=production

NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
ReadOnlyPaths=/etc/oidc-exchange
PrivateTmp=true

[Install]
WantedBy=multi-user.target
```

### 7. Install and start

```bash
sudo cp target/release/oidc-exchange /usr/local/bin/
sudo useradd --system --no-create-home oidc-exchange
sudo systemctl daemon-reload
sudo systemctl enable --now oidc-exchange
```

### 8. Reverse proxy

See the [generic Linux server guide](/deployment/linux-server/#reverse-proxy-nginx) for the nginx configuration. The reverse proxy setup is identical regardless of storage backend.

## Connection pool tuning

The `max_connections` setting in `[repository.postgres]` controls the sqlx connection pool size. Defaults to 5 if not specified. Guidelines:

- **Single instance**: 10-20 connections is typical
- **Multiple instances**: divide your PostgreSQL `max_connections` (minus overhead) across instances
- **Valkey sessions**: when using Valkey for sessions, PostgreSQL handles only user CRUD, so fewer connections are needed (5-10)

## Backup considerations

- **PostgreSQL**: standard `pg_dump` / WAL archiving covers all user data and (if not using Valkey) session data
- **Valkey sessions**: sessions are ephemeral by design (30-day default TTL). Valkey persistence (RDB/AOF) is optional; losing session data forces users to re-authenticate but does not lose accounts
