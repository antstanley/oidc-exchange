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
adapter = "stdout"
durability = "observe"

# Trust nginx's loopback connection before reading its forwarding chain.
[server]
trusted_proxies = ["127.0.0.1/32", "::1/128"]
trusted_proxy_hops = 1

[rate_limit]
enabled = true
store = "in_process"
window = "1m"
per_ip = 60
per_ip_failures = 10
per_subject = 10
per_provider = 600
max_concurrent_requests = 256
max_entries = 10000

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
        # Replace inbound forwarding data. oidc-exchange selects trusted hops from the
        # right, so nginx supplies the peer it observed as the rightmost (and only) value.
        proxy_set_header X-Forwarded-For $remote_addr;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
}
```

## Trusted forwarding

The `trusted_proxies` CIDRs must contain the address nginx uses to connect to the service. Enabled rate limiting with an empty trusted-proxy list emits a startup warning: that topology is safe for direct clients, but behind nginx it collapses every client into nginx's address budget. Health, discovery, and JWKS requests are excluded from authentication throttles; `/token` and `/revoke` retain both public and security budgets.
Only then does oidc-exchange accept `X-Forwarded-For`; it counts `trusted_proxy_hops` from the
right. Do not append an inbound client-supplied chain or rely on `X-Real-IP`: the service does
not use `X-Real-IP`, and a direct/untrusted forwarding header remains client-asserted rather
than a rate-limit key. Add one trusted hop and proxy CIDR for each proxy that appends a value.

## Log management

With `audit.adapter = "stdout"`, oidc-exchange writes structured audit JSON to stdout.
Systemd captures it in the journal; `telemetry.exporter = "stdout"` also writes tracing JSON:

```bash
journalctl -u oidc-exchange -f
```

Forward to your log aggregator via journald export, or switch to `exporter = "otlp"` to send traces directly to an OpenTelemetry collector.

## Specialized guides

For deployment-specific storage configurations, see:

- [Linux + PostgreSQL](/deployment/linux-postgres/) — relational storage with optional Valkey for sessions
- [Linux + SQLite](/deployment/linux-sqlite/) — embedded storage for single-server deployments
