---
title: Quick Start
description: Build and run oidc-exchange in 5 minutes.
---

## Prerequisites

- An OIDC provider (e.g., a [Google OAuth client](https://console.cloud.google.com/apis/credentials))

## Install

Choose one of the following methods:

### Option 1: Install script (recommended)

```bash
curl -fsSL https://raw.githubusercontent.com/antstanley/oidc-exchange/main/install.sh | sh
```

### Option 2: Docker

```bash
docker pull ghcr.io/antstanley/oidc-exchange:latest
```

### Option 3: npm

```bash
npm install @oidc-exchange/node
```

### Option 4: pip

```bash
pip install oidc-exchange
```

### Option 5: Build from source

Requires Rust 1.75+ and optionally [cargo-nextest](https://nexte.st) for testing.

```bash
cargo build --release
```

## Configure

Create a `config/default.toml`:

```toml
[server]
host = "0.0.0.0"
port = 8080
issuer = "https://auth.example.com"

[registration]
mode = "open"

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
adapter = "sqlite"

[repository.sqlite]
path = "./data/oidc-exchange.db"

[audit]
adapter = "noop"

[providers.google]
adapter = "oidc"
issuer = "https://accounts.google.com"
client_id = "${GOOGLE_CLIENT_ID}"
client_secret = "${GOOGLE_CLIENT_SECRET}"
scopes = ["openid", "email", "profile"]
```

## Generate a signing key

```bash
mkdir -p keys data
openssl genpkey -algorithm ed25519 -out keys/ed25519.pem
```

## Run

If you installed via the install script or built from source:

```bash
GOOGLE_CLIENT_ID=your-id GOOGLE_CLIENT_SECRET=your-secret \
  ./target/release/oidc-exchange
```

If you are using Docker:

```bash
docker run -p 8080:8080 \
  -v $(pwd)/config:/app/config:ro \
  -v $(pwd)/keys:/app/keys:ro \
  -e GOOGLE_CLIENT_ID=your-id \
  -e GOOGLE_CLIENT_SECRET=your-secret \
  ghcr.io/antstanley/oidc-exchange:latest
```

## Verify

```bash
# Health check
curl http://localhost:8080/health

# OpenID Connect discovery
curl http://localhost:8080/.well-known/openid-configuration

# JWKS endpoint
curl http://localhost:8080/keys
```

## Next steps

- [Configuration reference](/guides/configuration/) — all config options
- [API reference](/guides/api-reference/) — endpoints and request formats
- [Deployment guides](/deployment/overview/) — production deployment options
