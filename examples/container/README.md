---
title: Container Deployment Example
description: Deploy oidc-exchange using Docker Compose (local) or Kubernetes manifests.
version: "0.1"
last_updated: 2026-03-26
---

# Container Deployment Example

> **Tip:** Prebuilt Docker images are available at `ghcr.io/antstanley/oidc-exchange:latest`. See the [Docker guide](https://github.com/antstanley/oidc-exchange/blob/main/apps/website/src/content/docs/guides/docker.md) for details.

Generic container deployment with Docker Compose (for local development) and Kubernetes manifests for production-like environments. Uses DynamoDB Local for storage, which can be swapped for any supported backend.

## Prerequisites

- Docker
- Docker Compose
- kubectl (optional, for Kubernetes deployment)

## Docker Compose Quick Start

1. Build the image from the repository root:

   ```sh
   docker build -t oidc-exchange -f examples/container/Dockerfile .
   ```

2. Generate a signing key:

   ```sh
   mkdir -p examples/container/keys && openssl genpkey -algorithm ed25519 -out examples/container/keys/signing-key.pem
   ```

3. Start the services:

   ```sh
   cd examples/container && docker compose up
   ```

4. Test the deployment:

   ```sh
   curl http://localhost:8080/health
   ```

## Kubernetes

1. Build and push the image to your container registry.
2. Edit `k8s/secrets.yml` with base64-encoded values for your signing key and provider credentials.
3. Apply the manifests:

   ```sh
   kubectl apply -f k8s/
   ```

## Further Reading

See [docs/integration/container.md](../../docs/integration/container.md) for detailed integration guidance.
