---
title: Container Deployment
description: Deploying oidc-exchange in Docker, Kubernetes, and Cloud Run
version: "0.2"
last_updated: 2026-03-26
---

# Container Deployment

Run oidc-exchange as a long-lived container in ECS, EKS, Cloud Run, or any container orchestrator. The binary runs as an axum HTTP server when `AWS_LAMBDA_RUNTIME_API` is not set.

A runnable example with Docker Compose and Kubernetes manifests is in [`examples/container/`](../../examples/container/).

## Dockerfile

```dockerfile
FROM rust:1.85-slim AS builder
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/oidc-exchange /usr/local/bin/
COPY config/ /app/config/
EXPOSE 8080
ENV OIDC_EXCHANGE_ENV=production
CMD ["oidc-exchange"]
```

## Configuration

Create `config/production.toml`:

```toml
[server]
host = "0.0.0.0"
port = 8080
issuer = "https://auth.example.com"

[key_manager]
adapter = "local"

[key_manager.local]
private_key_path = "/etc/secrets/signing-key.pem"
algorithm = "EdDSA"
kid = "prod-1"

[repository]
adapter = "dynamodb"

[repository.dynamodb]
table_name = "oidc-exchange"
region = "us-east-1"

[audit]
adapter = "noop"

[telemetry]
enabled = true
exporter = "otlp"

[providers.google]
adapter = "oidc"
issuer = "https://accounts.google.com"
client_id = "${GOOGLE_CLIENT_ID}"
client_secret = "${GOOGLE_CLIENT_SECRET}"
scopes = ["openid", "email", "profile"]
```

## Key Management

For containers, you have flexibility in key management:

- **Local keys** — mount a signing key via a volume or Kubernetes secret. Use `adapter = "local"` with Ed25519 or ECDSA.
- **AWS KMS** — if running in AWS (ECS/EKS), use `adapter = "kms"` with IAM roles for service accounts or task roles.

## Docker Compose

```yaml
services:
  oidc-exchange:
    build: .
    ports:
      - "8080:8080"
    environment:
      OIDC_EXCHANGE_ENV: production
      GOOGLE_CLIENT_ID: ${GOOGLE_CLIENT_ID}
      GOOGLE_CLIENT_SECRET: ${GOOGLE_CLIENT_SECRET}
      AWS_REGION: us-east-1
    volumes:
      - ./keys:/etc/secrets:ro
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:8080/health"]
      interval: 30s
      timeout: 5s
      retries: 3

  dynamodb-local:
    image: amazon/dynamodb-local
    ports:
      - "8000:8000"
```

## ECS Fargate

Build and push the image to ECR:

```bash
docker build -t oidc-exchange .
docker tag oidc-exchange:latest <account-id>.dkr.ecr.us-east-1.amazonaws.com/oidc-exchange:latest
aws ecr get-login-password --region us-east-1 | docker login --username AWS --password-stdin <account-id>.dkr.ecr.us-east-1.amazonaws.com
docker push <account-id>.dkr.ecr.us-east-1.amazonaws.com/oidc-exchange:latest
```

In your task definition, set the key configuration:

- Use `adapter = "kms"` with a KMS key ARN and attach the appropriate IAM policy to the task role.
- Alternatively, inject a local signing key via AWS Secrets Manager into the container environment.

For production-grade ECS Fargate with auto-scaling and ALB, see the dedicated [ECS Fargate guide](ecs-fargate.md).

## Kubernetes

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: oidc-exchange
spec:
  replicas: 2
  selector:
    matchLabels:
      app: oidc-exchange
  template:
    metadata:
      labels:
        app: oidc-exchange
    spec:
      containers:
        - name: oidc-exchange
          image: your-registry/oidc-exchange:latest
          ports:
            - containerPort: 8080
          env:
            - name: OIDC_EXCHANGE_ENV
              value: production
            - name: GOOGLE_CLIENT_ID
              valueFrom:
                secretKeyRef:
                  name: oidc-exchange-secrets
                  key: google-client-id
            - name: GOOGLE_CLIENT_SECRET
              valueFrom:
                secretKeyRef:
                  name: oidc-exchange-secrets
                  key: google-client-secret
          volumeMounts:
            - name: signing-key
              mountPath: /etc/secrets
              readOnly: true
          livenessProbe:
            httpGet:
              path: /health
              port: 8080
            initialDelaySeconds: 5
            periodSeconds: 30
          readinessProbe:
            httpGet:
              path: /health
              port: 8080
            initialDelaySeconds: 2
            periodSeconds: 10
          resources:
            requests:
              cpu: 100m
              memory: 64Mi
            limits:
              cpu: 500m
              memory: 128Mi
      volumes:
        - name: signing-key
          secret:
            secretName: oidc-exchange-signing-key
---
apiVersion: v1
kind: Service
metadata:
  name: oidc-exchange
spec:
  selector:
    app: oidc-exchange
  ports:
    - port: 80
      targetPort: 8080
  type: ClusterIP
```

## Scaling

oidc-exchange is stateless (all state is in the configured database). Scale horizontally without coordination. Each instance holds an in-memory JWKS cache for upstream providers — this warms up on first request per provider and refreshes automatically.
