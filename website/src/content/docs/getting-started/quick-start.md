---
title: Quick Start
description: Build and run oidc-exchange in 5 minutes.
---

## Prerequisites

- Rust 1.75+
- [cargo-nextest](https://nexte.st) for testing (optional)
- An OIDC provider (e.g., a [Google OAuth client](https://console.cloud.google.com/apis/credentials))

## Build

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

```bash
GOOGLE_CLIENT_ID=your-id GOOGLE_CLIENT_SECRET=your-secret \
  ./target/release/oidc-exchange
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
