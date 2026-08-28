---
title: Quick Start
description: Build and run oidc-exchange in 5 minutes.
---

## Prerequisites

- An OIDC provider (e.g., a [Google OAuth client](https://console.cloud.google.com/apis/credentials))

## Install

Choose one of the following methods:

### Option 1: Verified install script (recommended)

For an authenticated binary install, install the [GitHub CLI](https://cli.github.com/) first. The installer requires the downloaded binary to have GitHub build provenance from repository `antstanley/oidc-exchange` and signer workflow `antstanley/oidc-exchange/.github/workflows/release.yml`:

```bash
command -v gh
curl -fsSL https://raw.githubusercontent.com/antstanley/oidc-exchange/main/install.sh | bash
```

To verify a manually downloaded binary, run:

```bash
gh attestation verify ./oidc-exchange-linux-x64 \
  --repo antstanley/oidc-exchange \
  --signer-workflow antstanley/oidc-exchange/.github/workflows/release.yml
```

Without `gh`, the installer loudly falls back to checksum-only corruption detection; that does not authenticate the release.

### Option 2: Verified GHCR container

The GHCR multi-arch tag has build provenance for its immutable final manifest digest (in addition to each platform digest). Verify the final GHCR manifest before running it:

```bash
gh attestation verify oci://ghcr.io/antstanley/oidc-exchange:latest \
  --repo antstanley/oidc-exchange \
  --signer-workflow antstanley/oidc-exchange/.github/workflows/release.yml
docker pull ghcr.io/antstanley/oidc-exchange:latest
```

This is GitHub build provenance, not a registry signature. The release is also copied to Docker Hub, but the workflow does not attach or promise a Docker Hub-verifiable attestation; use GHCR for this verification path.

### Option 3: npm

```bash
npm install @oidc-exchange/node
```

### Option 4: pip

```bash
pip install oidc-exchange
```

### Option 5: Build from source

Requires a recent stable Rust toolchain (CI builds and tests on rustc 1.98) and optionally [cargo-nextest](https://nexte.st) for testing.

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
# Origins Google's discovery document may name beyond the issuer's origin:
endpoint_origins = ["https://oauth2.googleapis.com", "https://www.googleapis.com"]
```

`endpoint_origins` pins which origins a provider's discovery document is allowed to name. Google serves its token and revocation endpoints from `oauth2.googleapis.com` and its JWKS URI from `www.googleapis.com`. See the [Identity Providers guide](/guides/providers/) for how origin pinning works.

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

- [Configuration reference](/guides/configuration/): all config options
- [API reference](/guides/api-reference/): endpoints and request formats
- [Deployment guides](/deployment/overview/): production deployment options
