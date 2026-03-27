---
title: "Storage Adapters"
description: "Available database, session, audit, and key management adapters."
---

oidc-exchange ships with all adapters compiled into a single binary. Configuration selects which adapters are active at runtime. This page details every available adapter and its configuration.

## User and session storage

The `[repository]` section selects the primary storage backend for both user records and session (refresh token) data. Three backends are available.

### DynamoDB

Single-table design optimized for the access patterns oidc-exchange uses: user lookup by ID, user lookup by external ID (provider subject), and session lookup by refresh token hash.

```toml
[repository]
adapter = "dynamodb"

[repository.dynamodb]
table_name = "oidc-exchange"
region = "us-east-1"       # optional, uses SDK default if omitted
```

DynamoDB is the recommended backend for AWS deployments, especially Lambda-based architectures where a connection pool is impractical. The table schema is defined in `schemas/dynamodb/table-design.json`.

### PostgreSQL

Relational storage using [sqlx](https://github.com/launchbadge/sqlx) with connection pooling. Suitable for teams that prefer SQL or need to query user data with ad-hoc SQL.

```toml
[repository]
adapter = "postgres"

[repository.postgres]
url = "postgres://user:pass@localhost:5432/oidc_exchange"
max_connections = 5
```

### SQLite

File-based storage using [sqlx](https://github.com/launchbadge/sqlx). Zero external dependencies --- ideal for single-server deployments or development.

```toml
[repository]
adapter = "sqlite"

[repository.sqlite]
path = "./data/oidc-exchange.db"
```

## Session-only storage

The optional `[session_repository]` section overrides the backend used for session and refresh token operations without affecting user storage. This allows you to pair a relational user store with a fast session store.

When `[session_repository]` is configured:
- User operations (create, get, update, delete) use the `[repository]` backend
- Session operations (store refresh token, lookup by hash, revoke) use the `[session_repository]` backend

When `[session_repository]` is not configured, all operations use the `[repository]` backend.

### Valkey / Redis

In-memory key-value store using the [fred](https://github.com/aembke/fred.rs) client. Provides sub-millisecond session lookups. Compatible with Redis, Valkey, and ElastiCache.

```toml
[session_repository]
adapter = "valkey"

[session_repository.valkey]
url = "redis://localhost:6379"
key_prefix = "oidc:"
```

### LMDB

Embedded key-value store using [heed](https://github.com/meilisearch/heed) (Rust bindings for LMDB). Fast local storage without a network dependency. Suitable for single-server deployments that need session performance beyond what SQLite offers.

```toml
[session_repository]
adapter = "lmdb"

[session_repository.lmdb]
path = "./lmdb"
max_size_mb = 64
```

## Common combinations

| Deployment | User storage | Session storage | Config |
|---|---|---|---|
| AWS Lambda | DynamoDB | DynamoDB (same) | `[repository] adapter = "dynamodb"` |
| ECS Fargate | DynamoDB | Valkey | Add `[session_repository] adapter = "valkey"` |
| Linux + PostgreSQL | PostgreSQL | PostgreSQL (same) | `[repository] adapter = "postgres"` |
| Linux + PostgreSQL + Valkey | PostgreSQL | Valkey | Add `[session_repository] adapter = "valkey"` |
| Linux + SQLite | SQLite | SQLite (same) | `[repository] adapter = "sqlite"` |
| Linux + SQLite + LMDB | SQLite | LMDB | Add `[session_repository] adapter = "lmdb"` |

## Key management

The `[key_manager]` section controls how access token JWTs are signed.

### Local key signing

Load a private key from disk and sign tokens in-process. Supports Ed25519 (EdDSA) and P-256 (ES256) keys.

```toml
[key_manager]
adapter = "local"

[key_manager.local]
private_key_path = "./keys/ed25519.pem"
algorithm = "EdDSA"        # "EdDSA" (Ed25519) or "ES256" (P-256)
kid = "key-1"
```

Generate a key:

```bash
# Ed25519
openssl genpkey -algorithm ed25519 -out keys/ed25519.pem

# P-256 (ECDSA)
openssl ecparam -name prime256v1 -genkey -noout -out keys/p256.pem
```

Local key management is suitable for development and single-server deployments. For production, consider KMS for automatic key protection and access control.

### AWS KMS

Sign tokens using an AWS KMS asymmetric key (ECC_NIST_P256). The private key never leaves KMS --- signing is a remote API call.

```toml
[key_manager]
adapter = "kms"

[key_manager.kms]
key_id = "arn:aws:kms:us-east-1:123456789:key/abcd-1234"
algorithm = "ECDSA_SHA_256"
kid = "prod-key-1"
```

KMS handles key rotation transparently. The service uses standard AWS SDK credential resolution (environment variables, instance profile, ECS task role, etc.).

## Audit logging

The `[audit]` section controls where compliance and security events are sent. Every token exchange, refresh, revocation, registration denial, and user lifecycle event generates an audit record.

### Noop

Events are not sent to any external system. When the audit provider is down or absent, events are always written to stdout (info and below) or stderr (error and above) as structured JSON --- this fallback happens regardless of adapter.

```toml
[audit]
adapter = "noop"
blocking_threshold = "warning"
```

### CloudTrail Lake

Send audit events to AWS CloudTrail Lake for long-term compliance storage and SQL-based querying.

```toml
[audit]
adapter = "cloudtrail"
blocking_threshold = "warning"

[audit.cloudtrail]
channel_arn = "arn:aws:cloudtrail:us-east-1:123456789:channel/my-channel"
```

### SQS

Send audit events to an SQS queue. Useful for building a pipeline to S3, Iceberg, or other analytics backends via Firehose or Lambda.

```toml
[audit]
adapter = "sqs"
blocking_threshold = "warning"

[audit.sqs]
queue_url = "https://sqs.us-east-1.amazonaws.com/123456789/audit-queue"
```

### Blocking threshold

The `blocking_threshold` setting controls what happens when the audit provider fails. Audit events have syslog severity levels (RFC 5424): emergency, alert, critical, error, warning, notice, info, debug.

If the audit provider fails to emit an event and the event's severity is at or above the configured threshold, the operation that triggered the event also fails. Events below the threshold are logged to stdout/stderr as a fallback and the operation proceeds.

For example, with `blocking_threshold = "warning"`:
- A failed `TokenExchange` audit (severity: notice) logs to stdout and the token exchange succeeds
- A failed `RegistrationDenied` audit (severity: warning) causes the request to fail with a 500 error

## User sync

The `[user_sync]` section enables outbound notifications when users are created, updated, or deleted.

### Webhook

Sends HTTP POST requests with HMAC-SHA256 signed payloads to an external URL.

```toml
[user_sync]
enabled = true
adapter = "webhook"

[user_sync.webhook]
url = "https://internal-api.example.com/user-events"
secret = "${SYNC_WEBHOOK_SECRET}"
timeout = "5s"
retries = 2
```

The webhook payload:

```json
{
  "event": "user.created",
  "timestamp": "2026-03-24T10:00:00Z",
  "data": { }
}
```

Event types: `user.created`, `user.updated`, `user.deleted`. The request includes an `X-Signature-256` header containing the hex-encoded HMAC-SHA256 of the raw request body.

User sync is non-blocking: sync failures are logged via `tracing::warn!` and never fail the originating request.

### Noop

Disables user sync. This is the default when `user_sync.enabled` is `false` or the section is omitted.
