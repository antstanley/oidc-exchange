# oidc-exchange

Validate ID tokens from third-party OIDC providers (Google, Apple, …) and exchange them for self-issued access and refresh tokens — a single Rust binary that runs as a long-lived HTTP server (or an AWS Lambda function).

## Image

Multi-arch (`linux/amd64`, `linux/arm64`), published to both registries:

- Docker Hub — `antstanley80/oidc-exchange`
- GHCR — `ghcr.io/antstanley/oidc-exchange`

Tags: `latest`, `X.Y.Z`, `X.Y`, `X`.

The GHCR multi-arch tag has build provenance for its immutable final manifest digest (in addition to each platform digest). Verify the final GHCR manifest before running it:

```bash
gh attestation verify oci://ghcr.io/antstanley/oidc-exchange:latest \
  --repo antstanley/oidc-exchange \
  --signer-workflow antstanley/oidc-exchange/.github/workflows/release.yml
docker pull ghcr.io/antstanley/oidc-exchange:latest
```

This is GitHub build provenance, not a registry signature. The release is also copied to Docker Hub, but the workflow does not attach or promise a Docker Hub-verifiable attestation; use GHCR for this verification path.

## Run

```bash
docker run -p 8080:8080 \
  -v "$(pwd)/config.toml:/app/config.toml:ro" \
  antstanley80/oidc-exchange:latest
```

The server listens on the port from your config (default `8080`) and exposes `/token`, `/revoke`, `/keys`, `/.well-known/openid-configuration`, and `/health`.

## Configure

Mount a `config.toml` (as above) and/or override values with environment variables (`OIDC_EXCHANGE__{section}__{key}`); `${VAR}` placeholders in the TOML resolve from the environment. Minimal example:

```toml
[server]
host = "0.0.0.0"
port = 8080
issuer = "https://auth.example.com"

[providers.google]
adapter = "oidc"
issuer = "https://accounts.google.com"
client_id = "${GOOGLE_CLIENT_ID}"
client_secret = "${GOOGLE_CLIENT_SECRET}"
```

See the [full configuration guide](https://github.com/antstanley/oidc-exchange#configuration) and the [deployment guides](https://github.com/antstanley/oidc-exchange/tree/main/docs/integration) (ECS Fargate, generic container / Kubernetes, …).

## Links

- [Repository & full docs](https://github.com/antstanley/oidc-exchange)
- [Why oidc-exchange?](https://github.com/antstanley/oidc-exchange#why-oidc-exchange)

Images are built multi-arch on native runners and published from CI. MIT licensed.
