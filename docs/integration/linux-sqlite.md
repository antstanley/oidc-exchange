---
title: Linux Server with SQLite
description: Deploy oidc-exchange on a Linux server with SQLite for embedded storage, with optional LMDB for sessions.
version: "0.2"
last_updated: 2026-03-26
---

# Linux Server with SQLite

Run oidc-exchange on a single Linux host using SQLite for all persistent storage. No external database services needed. This is the simplest production deployment — one binary, one config file, one database file.

Optionally, use LMDB for session storage when you want faster session lookups without adding a network service.

A runnable example with setup scripts and configs is in [`examples/linux-sqlite/`](../../examples/linux-sqlite/).

## When to use this

- Single-server deployment with no external database dependencies
- Low to moderate traffic (hundreds of requests per second)
- You want the simplest possible ops story: back up one directory, restore anywhere
- Optionally: use LMDB for faster session reads on the same server

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
              │   SQLite   │  │    LMDB    │
              │  (users)   │  │ (sessions)  │
              └────────────┘  └─────────────┘
                                 optional
```

Without LMDB, SQLite handles both users and sessions.

## Prerequisites

- A Linux server with the oidc-exchange binary — install via the [install script](README.md#installing), download a [prebuilt release](https://github.com/antstanley/oidc-exchange/releases), or [build from source](README.md#building-from-source)
- A writable directory for the SQLite database file
- (Optional) A writable directory for the LMDB environment

## Step-by-step

### 1. Create directories

```bash
sudo mkdir -p /var/lib/oidc-exchange/data
sudo mkdir -p /etc/oidc-exchange
sudo chown -R oidc-exchange:oidc-exchange /var/lib/oidc-exchange
```

If using LMDB for sessions:

```bash
sudo mkdir -p /var/lib/oidc-exchange/lmdb
sudo chown oidc-exchange:oidc-exchange /var/lib/oidc-exchange/lmdb
```

### 2. Generate a signing key

```bash
openssl genpkey -algorithm ed25519 -out /etc/oidc-exchange/signing-key.pem
chmod 600 /etc/oidc-exchange/signing-key.pem
```

### 3. Configure (SQLite only)

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
adapter = "sqlite"

[repository.sqlite]
path = "/var/lib/oidc-exchange/data/oidc-exchange.db"

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

SQLite runs with WAL journal mode and foreign keys enabled automatically. The database file and tables are created on first startup.

### 4. Configure (SQLite + LMDB for sessions)

LMDB is an embedded key-value store optimized for read-heavy workloads. Session lookups (every token refresh) are the hottest path in oidc-exchange — LMDB serves these from memory-mapped files with zero-copy reads.

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

# Users in SQLite
[repository]
adapter = "sqlite"

[repository.sqlite]
path = "/var/lib/oidc-exchange/data/oidc-exchange.db"

# Sessions in LMDB
[session_repository]
adapter = "lmdb"

[session_repository.lmdb]
path = "/var/lib/oidc-exchange/lmdb"
max_size_mb = 256

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

The `max_size_mb` setting controls the LMDB memory map size. 256 MB is generous for session data — each session is roughly 500 bytes, so 256 MB supports ~500,000 concurrent sessions. The space is reserved but not allocated until used.

### 5. Create the environment file

```bash
cat > /etc/oidc-exchange/env <<'EOF'
GOOGLE_CLIENT_ID=your-client-id
GOOGLE_CLIENT_SECRET=your-client-secret
EOF

chmod 600 /etc/oidc-exchange/env
```

### 6. Create the systemd service

```ini
# /etc/systemd/system/oidc-exchange.service

[Unit]
Description=oidc-exchange token service
After=network-online.target
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
ReadWritePaths=/var/lib/oidc-exchange
PrivateTmp=true

[Install]
WantedBy=multi-user.target
```

Note `ReadWritePaths=/var/lib/oidc-exchange` — the service needs write access to the SQLite and LMDB data directories.

### 7. Install and start

```bash
sudo cp target/release/oidc-exchange /usr/local/bin/
sudo useradd --system --no-create-home oidc-exchange
sudo systemctl daemon-reload
sudo systemctl enable --now oidc-exchange
```

### 8. Reverse proxy

See the [generic Linux server guide](linux-server.md#reverse-proxy-nginx) for the nginx configuration.

## Backup

All state lives in `/var/lib/oidc-exchange/`. To back up:

```bash
# SQLite — use the .backup command for a consistent snapshot
sqlite3 /var/lib/oidc-exchange/data/oidc-exchange.db ".backup /tmp/oidc-exchange-backup.db"
```

For LMDB, copy the directory while the service is running — LMDB's copy-on-write design means readers never block writers and file copies are crash-consistent. Alternatively, stop the service and copy `/var/lib/oidc-exchange/lmdb/`.

LMDB session data is ephemeral (sessions expire). Losing it only forces users to re-authenticate.

## Limitations

- **Single-server only** — SQLite and LMDB do not support concurrent access from multiple processes on different hosts. For multi-server deployments, use [PostgreSQL](linux-postgres.md) or [DynamoDB](aws-lambda.md).
- **Write throughput** — SQLite with WAL handles hundreds of writes per second. If you need thousands, consider PostgreSQL.
- **No horizontal scaling** — you cannot add more oidc-exchange instances behind a load balancer with SQLite/LMDB. Each instance would have its own database.
