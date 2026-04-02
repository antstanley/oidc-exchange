---
title: Docker
description: Run oidc-exchange in Docker with GHCR or Docker Hub images.
---

## Pull the Image

From GitHub Container Registry:

```bash
docker pull ghcr.io/antstanley/oidc-exchange:latest
```

From Docker Hub:

```bash
docker pull antstanley/oidc-exchange:latest
```

## Run with Config Volume

Mount your configuration directory into the container:

```bash
docker run -p 8080:8080 \
  -v ./config:/app/config \
  ghcr.io/antstanley/oidc-exchange:latest
```

Environment variables can be passed with `-e`:

```bash
docker run -p 8080:8080 \
  -v ./config:/app/config \
  -e OIDC_EXCHANGE__SERVER__ISSUER=https://auth.example.com \
  -e GOOGLE_CLIENT_SECRET=your-secret \
  ghcr.io/antstanley/oidc-exchange:latest
```

## Docker Compose

```yaml
# docker-compose.yml
version: "3.9"

services:
  oidc-exchange:
    image: ghcr.io/antstanley/oidc-exchange:latest
    ports:
      - "8080:8080"
    volumes:
      - ./config:/app/config
    environment:
      - OIDC_EXCHANGE_ENV=production
      - OIDC_EXCHANGE__SERVER__ISSUER=https://auth.example.com
    restart: unless-stopped

  postgres:
    image: postgres:16-alpine
    environment:
      POSTGRES_DB: oidc_exchange
      POSTGRES_USER: oidc
      POSTGRES_PASSWORD: changeme
    volumes:
      - pgdata:/var/lib/postgresql/data
    ports:
      - "5432:5432"

volumes:
  pgdata:
```

To use PostgreSQL as the storage backend, update your config to point at the `postgres` service:

```toml
[storage]
adapter = "postgres"
url = "postgresql://oidc:changeme@postgres:5432/oidc_exchange"
```

## Multi-Architecture Support

Images are published for both architectures:

- `linux/amd64`
- `linux/arm64`

Docker automatically pulls the correct image for your platform. To explicitly specify an architecture:

```bash
docker pull --platform linux/arm64 ghcr.io/antstanley/oidc-exchange:latest
```

## Image Tags

| Tag | Description |
|-----|-------------|
| `latest` | Most recent stable release |
| `v1.0.0` | Exact version pin |
| `v1.0` | Latest patch release in the 1.0.x line |
| `v1` | Latest minor and patch release in the 1.x line |

Pin to an exact version in production for reproducible deployments:

```yaml
image: ghcr.io/antstanley/oidc-exchange:v1.0.0
```

## Custom Dockerfile

Extend the prebuilt image to bundle your own configuration:

```dockerfile
FROM ghcr.io/antstanley/oidc-exchange:latest

COPY config/ /app/config/

ENV OIDC_EXCHANGE_ENV=production
```

Build and run:

```bash
docker build -t my-oidc-exchange .
docker run -p 8080:8080 my-oidc-exchange
```

This is useful for CI/CD pipelines where you want the config baked into the image rather than mounted at runtime.
