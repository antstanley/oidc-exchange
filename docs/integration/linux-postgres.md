---
title: Linux Server with PostgreSQL
description: Deploy oidc-exchange on a Linux server with PostgreSQL for user and session storage, with optional Valkey/Redis for sessions.
version: "0.2"
last_updated: 2026-03-26
---

# Linux Server with PostgreSQL

Run oidc-exchange on a Linux host using PostgreSQL for persistent storage. This guide covers PostgreSQL for both users and sessions, with an optional Valkey/Redis upgrade for session storage when you need lower-latency token operations.

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

- A Linux server with oidc-exchange binary (see [build instructions](README.md#prerequisites))
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

oidc-exchange runs its own migrations on startup. The tables created are:

```sql
CREATE TABLE users (
    id              TEXT PRIMARY KEY,
    external_id     TEXT NOT NULL,
    provider        TEXT NOT NULL,
    email           TEXT,
    display_name    TEXT,
    metadata        JSONB NOT NULL DEFAULT '{}',
    claims          JSONB NOT NULL DEFAULT '{}',
    status          TEXT NOT NULL DEFAULT 'active',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX idx_users_external_id ON users (external_id);

CREATE TABLE sessions (
    refresh_token_hash TEXT PRIMARY KEY,
    user_id            TEXT NOT NULL,
    provider           TEXT NOT NULL,
    expires_at         TIMESTAMPTZ NOT NULL,
    device_id          TEXT,
    user_agent         TEXT,
    ip_address         TEXT,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_sessions_user_id ON sessions (user_id);
```

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
adapter = "noop"

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
adapter = "noop"

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

See the [generic Linux server guide](linux-server.md#reverse-proxy-nginx) for the nginx configuration. The reverse proxy setup is identical regardless of storage backend.

## Connection pool tuning

The `max_connections` setting in `[repository.postgres]` controls the sqlx connection pool size. Defaults to 5 if not specified. Guidelines:

- **Single instance**: 10-20 connections is typical
- **Multiple instances**: divide your PostgreSQL `max_connections` (minus overhead) across instances
- **Valkey sessions**: when using Valkey for sessions, PostgreSQL handles only user CRUD — fewer connections needed (5-10)

## Backup considerations

- **PostgreSQL**: standard `pg_dump` / WAL archiving covers all user data and (if not using Valkey) session data
- **Valkey sessions**: sessions are ephemeral by design (30-day default TTL). Valkey persistence (RDB/AOF) is optional — losing session data forces users to re-authenticate but does not lose accounts
