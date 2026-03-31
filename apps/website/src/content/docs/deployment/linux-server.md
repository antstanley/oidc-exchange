---
title: Generic Linux Server
description: Deploy oidc-exchange on a Linux host with systemd and nginx.
---

Run oidc-exchange directly on a Linux host behind a reverse proxy. This is the simplest deployment model for on-prem or single-server setups.

A runnable example is in [`examples/linux-server/`](https://github.com/example/oidc-exchange/tree/main/examples/linux-server).

## Build

```bash
cargo build --release
```

Cross-compile for a different target if needed:

```bash
# For x86_64 Linux from macOS
rustup target add x86_64-unknown-linux-gnu
cargo build --release --target x86_64-unknown-linux-gnu
```

## Signing key

```bash
openssl genpkey -algorithm ed25519 -out /etc/oidc-exchange/signing-key.pem
chmod 600 /etc/oidc-exchange/signing-key.pem
```

## Configuration

Place config files in `/etc/oidc-exchange/config/`:

```toml
# /etc/oidc-exchange/config/default.toml

[server]
host = "127.0.0.1"
port = 8080
issuer = "https://auth.example.com"

[key_manager]
adapter = "local"

[key_manager.local]
private_key_path = "/etc/oidc-exchange/signing-key.pem"
algorithm = "EdDSA"
kid = "server-1"

[repository]
adapter = "dynamodb"

[repository.dynamodb]
table_name = "oidc-exchange"
region = "us-east-1"

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

Bind to `127.0.0.1` and put a reverse proxy (nginx, Caddy) in front for TLS termination.

## systemd service

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

# Environment
EnvironmentFile=/etc/oidc-exchange/env
Environment=OIDC_EXCHANGE_ENV=production

# Hardening
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
ReadOnlyPaths=/etc/oidc-exchange
PrivateTmp=true

[Install]
WantedBy=multi-user.target
```

Create the environment file with secrets:

```bash
# /etc/oidc-exchange/env
GOOGLE_CLIENT_ID=your-client-id
GOOGLE_CLIENT_SECRET=your-client-secret
```

```bash
chmod 600 /etc/oidc-exchange/env
```

## Install and start

```bash
sudo cp target/release/oidc-exchange /usr/local/bin/
sudo useradd --system --no-create-home oidc-exchange
sudo systemctl daemon-reload
sudo systemctl enable --now oidc-exchange
```

## Reverse proxy (nginx)

```nginx
upstream oidc_exchange {
    server 127.0.0.1:8080;
}

server {
    listen 443 ssl;
    server_name auth.example.com;

    ssl_certificate /etc/ssl/certs/auth.example.com.pem;
    ssl_certificate_key /etc/ssl/private/auth.example.com.key;

    location / {
        proxy_pass http://oidc_exchange;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
}
```

## Log management

With `exporter = "stdout"`, oidc-exchange writes structured JSON logs to stdout. Systemd captures these in the journal:

```bash
journalctl -u oidc-exchange -f
```

Forward to your log aggregator via journald export, or switch to `exporter = "otlp"` to send traces directly to an OpenTelemetry collector.

## Specialized guides

For deployment-specific storage configurations, see:

- [Linux + PostgreSQL](/deployment/linux-postgres/) — relational storage with optional Valkey for sessions
- [Linux + SQLite](/deployment/linux-sqlite/) — embedded storage for single-server deployments
