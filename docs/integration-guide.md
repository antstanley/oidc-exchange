---
title: Integration Guide
description: How to deploy and integrate oidc-exchange in AWS serverless, container, and generic Linux server environments.
version: "0.1"
last_updated: 2026-03-26
---

# Integration Guide

This guide covers deploying oidc-exchange in three environments: AWS serverless (Lambda), container-based, and generic Linux server. All three use the same binary — the deployment target is determined by runtime detection and configuration.

## Prerequisites

All environments require:

- A built `oidc-exchange` binary (see [Building](#building))
- A TOML configuration file (see [Configuration](#configuration))
- At least one OIDC provider configured (Google, Apple, etc.)
- A DynamoDB table or compatible storage backend

### Building

```bash
# Standard server/container binary
cargo build --release
# Output: target/release/oidc-exchange

# AWS Lambda binary (requires cargo-lambda)
cargo lambda build --release
# Output: target/lambda/oidc-exchange/bootstrap
```

### Configuration

oidc-exchange loads configuration in order:

1. `config/default.toml` — baseline defaults
2. `config/{OIDC_EXCHANGE_ENV}.toml` — environment-specific overrides
3. Environment variables — `OIDC_EXCHANGE__{section}__{key}` (double underscore delimiters)
4. `${VAR_NAME}` placeholders — resolved from environment at load time

Secrets (client secrets, API keys) should always use `${VAR_NAME}` placeholders and be injected via environment variables, never hardcoded in TOML files.

---

## AWS Serverless (Lambda)

This is the recommended deployment for AWS-native workloads. oidc-exchange detects the `AWS_LAMBDA_RUNTIME_API` environment variable at startup and automatically switches to Lambda mode — no code or config changes needed.

### Infrastructure

A complete CDK example is in `examples/aws-web/infra/`. The key resources are:

| Resource | Purpose |
|----------|---------|
| DynamoDB table | User and session storage (single-table design, on-demand billing) |
| KMS key (ECC_NIST_P256) | JWT signing — keys never leave KMS |
| CloudTrail Lake event data store | Audit trail (optional) |
| Lambda function (`provided.al2023`) | The oidc-exchange binary |
| API Gateway HTTP API | Routes traffic to Lambda |

### Step-by-step

**1. Build the Lambda bootstrap binary**

```bash
cargo lambda build --release
```

This produces `target/lambda/oidc-exchange/bootstrap`, a statically linked binary compatible with the `provided.al2023` Lambda runtime.

**2. Create the DynamoDB table**

The table uses a single-table design with a partition key (`PK`), sort key (`SK`), and one global secondary index (`GSI1`). See `schemas/dynamodb/table-design.json` for the full schema.

```bash
aws dynamodb create-table \
  --table-name oidc-exchange \
  --attribute-definitions \
    AttributeName=PK,AttributeType=S \
    AttributeName=SK,AttributeType=S \
    AttributeName=GSI1pk,AttributeType=S \
    AttributeName=GSI1sk,AttributeType=S \
  --key-schema \
    AttributeName=PK,KeyType=HASH \
    AttributeName=SK,KeyType=RANGE \
  --global-secondary-indexes \
    'IndexName=GSI1,KeySchema=[{AttributeName=GSI1pk,KeyType=HASH},{AttributeName=GSI1sk,KeyType=RANGE}],Projection={ProjectionType=ALL}' \
  --billing-mode PAY_PER_REQUEST \
  --time-to-live-specification 'Enabled=true,AttributeName=ttl'
```

**3. Create the KMS signing key**

```bash
aws kms create-key \
  --key-spec ECC_NIST_P256 \
  --key-usage SIGN_VERIFY \
  --description "oidc-exchange JWT signing key"
```

Note the key ARN for the configuration.

**4. Configure**

Create `config/lambda.toml`:

```toml
[server]
issuer = "https://auth.example.com"

[key_manager]
adapter = "kms"

[key_manager.kms]
key_id = "${KMS_KEY_ARN}"
algorithm = "ECDSA_SHA256"
kid = "prod-1"

[repository]
adapter = "dynamodb"

[repository.dynamodb]
table_name = "oidc-exchange"

[audit]
adapter = "cloudtrail"
blocking_threshold = "error"

[audit.cloudtrail]
channel_arn = "${CLOUDTRAIL_CHANNEL_ARN}"

[providers.google]
adapter = "oidc"
issuer = "https://accounts.google.com"
client_id = "${GOOGLE_CLIENT_ID}"
client_secret = "${GOOGLE_CLIENT_SECRET}"
scopes = ["openid", "email", "profile"]
```

**5. Deploy the Lambda function**

```bash
# Copy the bootstrap binary
cp target/lambda/oidc-exchange/bootstrap deploy/

# Create the function
aws lambda create-function \
  --function-name oidc-exchange \
  --runtime provided.al2023 \
  --handler bootstrap \
  --architectures arm64 \
  --zip-file fileb://deploy/bootstrap.zip \
  --role arn:aws:iam::123456789012:role/oidc-exchange-role \
  --memory-size 256 \
  --timeout 29 \
  --environment "Variables={OIDC_EXCHANGE_ENV=lambda,GOOGLE_CLIENT_ID=...,GOOGLE_CLIENT_SECRET=...,KMS_KEY_ARN=...,CLOUDTRAIL_CHANNEL_ARN=...}"
```

Or use the CDK stack in `examples/aws-web/infra/` for a fully automated deployment:

```bash
cd examples/aws-web/infra
npm install
npx cdk deploy \
  -c googleClientId="your-client-id" \
  -c googleClientSecret="your-client-secret"
```

**6. Create the API Gateway route**

Route `/auth/{proxy+}` to the Lambda function. The CDK example does this automatically. For manual setup:

```bash
aws apigatewayv2 create-api \
  --name oidc-exchange \
  --protocol-type HTTP

# Add Lambda integration and routes for /auth/{proxy+}
```

### IAM permissions

The Lambda execution role needs:

```json
{
  "Effect": "Allow",
  "Action": [
    "dynamodb:GetItem",
    "dynamodb:PutItem",
    "dynamodb:UpdateItem",
    "dynamodb:DeleteItem",
    "dynamodb:Query",
    "dynamodb:BatchWriteItem"
  ],
  "Resource": [
    "arn:aws:dynamodb:*:*:table/oidc-exchange",
    "arn:aws:dynamodb:*:*:table/oidc-exchange/index/GSI1"
  ]
}
```

Plus `kms:Sign`, `kms:GetPublicKey` on the signing key, and optionally `cloudtrail-data:PutAuditEvents` on the CloudTrail channel.

### Cold start considerations

The `provided.al2023` runtime with a Rust binary typically cold-starts in 50-150ms. KMS signing adds ~20ms per request. To minimize cold starts:

- Use ARM64 (`arm64` architecture) for lower cost and comparable performance
- Set memory to 256 MB or higher — Lambda allocates CPU proportionally to memory
- Use provisioned concurrency if sub-100ms p99 latency is required

---

## Container-based

Run oidc-exchange as a long-lived container in ECS, EKS, Cloud Run, or any container orchestrator. The binary runs as an axum HTTP server when `AWS_LAMBDA_RUNTIME_API` is not set.

### Dockerfile

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

### Configuration for containers

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

For containers, you have flexibility in key management:

- **Local keys** — mount a signing key via a volume or Kubernetes secret. Use `adapter = "local"` with Ed25519 or ECDSA.
- **AWS KMS** — if running in AWS (ECS/EKS), use `adapter = "kms"` with IAM roles for service accounts or task roles.

### Docker Compose example

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

### ECS Fargate

```bash
# Build and push to ECR
docker build -t oidc-exchange .
aws ecr get-login-password | docker login --username AWS --password-stdin $ECR_URI
docker tag oidc-exchange:latest $ECR_URI/oidc-exchange:latest
docker push $ECR_URI/oidc-exchange:latest
```

Key ECS task definition settings:

- **CPU/Memory**: 256 CPU / 512 MB is sufficient for most workloads
- **Health check**: `GET /health` on port 8080
- **Secrets**: Use AWS Secrets Manager references for `GOOGLE_CLIENT_SECRET` and signing keys
- **Task role**: DynamoDB and KMS permissions (same as the Lambda IAM policy above)

### Kubernetes

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

### Scaling considerations

oidc-exchange is stateless (all state is in DynamoDB). Scale horizontally without coordination. Each instance holds an in-memory JWKS cache for upstream providers — this warms up on first request per provider and refreshes automatically.

---

## Generic Linux Server

Run oidc-exchange directly on a Linux host behind a reverse proxy. This is the simplest deployment model for on-prem or single-server setups.

### Step-by-step

**1. Build the binary**

```bash
cargo build --release
```

Cross-compile for a different target if needed:

```bash
# For x86_64 Linux from macOS
rustup target add x86_64-unknown-linux-gnu
cargo build --release --target x86_64-unknown-linux-gnu
```

**2. Generate a signing key**

```bash
openssl genpkey -algorithm ed25519 -out /etc/oidc-exchange/signing-key.pem
chmod 600 /etc/oidc-exchange/signing-key.pem
```

**3. Create the configuration**

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

**4. Create a systemd service**

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

**5. Install and start**

```bash
sudo cp target/release/oidc-exchange /usr/local/bin/
sudo useradd --system --no-create-home oidc-exchange
sudo systemctl daemon-reload
sudo systemctl enable --now oidc-exchange
```

**6. Reverse proxy (nginx)**

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

### Log management

With `exporter = "stdout"`, oidc-exchange writes structured JSON logs to stdout. Systemd captures these in the journal:

```bash
journalctl -u oidc-exchange -f
```

Forward to your log aggregator via journald export, or switch to `exporter = "otlp"` to send traces directly to an OpenTelemetry collector.

---

## Client Integration

Regardless of deployment method, clients interact with oidc-exchange the same way.

### Token exchange

Your client application handles the OAuth flow with the identity provider (Google, Apple, etc.) and sends the resulting authorization code to oidc-exchange:

```bash
curl -X POST https://auth.example.com/token \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d "grant_type=authorization_code" \
  -d "code=AUTH_CODE_FROM_PROVIDER" \
  -d "provider=google" \
  -d "redirect_uri=https://app.example.com/callback"
```

Response:

```json
{
  "access_token": "eyJhbGciOi...",
  "refresh_token": "dGhpcyBpcyBh...",
  "token_type": "Bearer",
  "expires_in": 900
}
```

### Token refresh

```bash
curl -X POST https://auth.example.com/token \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d "grant_type=refresh_token" \
  -d "refresh_token=dGhpcyBpcyBh..."
```

### Token revocation

```bash
curl -X POST https://auth.example.com/revoke \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d "token=dGhpcyBpcyBh..."
```

### JWKS verification

Downstream services verify access tokens by fetching the public key from the JWKS endpoint:

```bash
curl https://auth.example.com/keys
```

Most JWT libraries support JWKS URLs natively. Point your verification middleware at `https://auth.example.com/keys` and it will cache and rotate keys automatically.

### OpenID Connect discovery

```bash
curl https://auth.example.com/.well-known/openid-configuration
```

This returns the standard discovery document, including the JWKS URI, supported grant types, and token endpoint.
