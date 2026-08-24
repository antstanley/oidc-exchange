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
algorithm = "ES256"
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

### Stdout/Stderr

Audit events are emitted as structured JSON to the process output --- info-and-below to stdout, error-and-above to stderr.

```toml
[audit]
adapter = "stdout"
blocking_threshold = "warning"
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

Event types: `user.created`, `user.updated`, `user.deleted`. User sync is non-blocking: sync failures are logged via `tracing::warn!` and never fail the originating request.

### Verifying a delivery (receiver contract)

Each delivery is authenticated and identified by three headers:

| Header | Value |
|---|---|
| `X-Webhook-Timestamp` | The RFC3339 instant the delivery was minted |
| `X-Webhook-Delivery-Id` | A ULID unique to the delivery occasion |
| `X-Signature-256` | `sha256=` followed by the hex HMAC-SHA256 of `<X-Webhook-Timestamp> "." <X-Webhook-Delivery-Id> "." <raw request body>` under your configured secret |

The signature and delivery id are minted **once** per logical delivery, outside
the retry loop: every attempt in a retry burst carries the same id, timestamp,
and signature, and byte-identical bodies. A repeated `X-Webhook-Delivery-Id` is
therefore a retry of one delivery — treat it as such, not as an anomaly.

A conforming receiver **must**:

1. Verify the signature **before parsing the body**, which makes
   `X-Webhook-Timestamp` an authenticated value.
2. Reject deliveries whose `X-Webhook-Timestamp` is outside ±5 minutes of the
   receiver's clock. This bounds replay of a captured delivery to the tolerance
   window.
3. Deduplicate on `X-Webhook-Delivery-Id`, retaining seen ids for at least that
   ±5-minute window — at least as long as timestamps are trusted, so no expired
   delivery can be replayed past the dedup memory.
4. Treat any 2xx as success. 5xx and timeout responses are retried by the sender
   up to the configured `retries` count with exponential backoff; 4xx is not
   retried.

Worked receiver example (Node.js):

```js
import { createHmac, timingSafeEqual } from "node:crypto";

export function verifyDelivery(req, rawBody, secret, seenIds, now = Date.now()) {
  // 1. Signature first, over exactly the documented input — before any JSON.parse.
  const expected =
    "sha256=" +
    createHmac("sha256", secret)
      .update(`${req.header("x-webhook-timestamp")}.${req.header("x-webhook-delivery-id")}.${rawBody}`)
      .digest("hex");
  const received = req.header("x-signature-256");
  if (
    typeof received !== "string" ||
    received.length !== expected.length ||
    !timingSafeEqual(Buffer.from(received), Buffer.from(expected))
  ) {
    return { ok: false, reason: "bad signature" };
  }

  // 2. Freshness: reject anything outside the ±5 minute tolerance.
  const sentAt = Date.parse(req.header("x-webhook-timestamp"));
  const TOLERANCE_MS = 5 * 60 * 1000; // keep this constant paired with the dedup window below
  if (!Number.isFinite(sentAt) || Math.abs(now - sentAt) > TOLERANCE_MS) {
    return { ok: false, reason: "stale or future timestamp" };
  }

  // 3. Dedup: one id is one delivery; repeats are retries, not new events.
  const deliveryId = req.header("x-webhook-delivery-id");
  if (seenIds.has(deliveryId)) {
    return { ok: true, reason: "retry of a delivered id — acknowledged, not reprocessed" };
  }
  seenIds.add(deliveryId); // retain ids for AT LEAST the tolerance window

  // 4. Only now parse and act on the body.
  const event = JSON.parse(rawBody);
  return { ok: true, event };
}
```

#### Release note (breaking receiver change)

**Webhook receivers must be updated when deploying this version.** Both the
signed input and the `X-Signature-256` value format changed:

- Before: `X-Signature-256` carried the bare hex HMAC-SHA256 of the raw body only.
- After: the header carries `sha256=<hex>` over `timestamp.delivery-id.body`, and
  receivers must additionally check `X-Webhook-Timestamp` freshness (±5 minutes)
  and deduplicate on `X-Webhook-Delivery-Id`.

Every existing receiver rejects every delivery until it is updated — there is no
negotiation or compatibility mode. The failure is quiet on the receiver side (a
4xx is not retried, and sync failures are logged-and-swallowed upstream), so plan
the receiver deploy together with this upgrade. `user_sync.enabled` defaults to
`false` and no shipped example enables it; see the worked example above for the
reference verification flow.

#### Release note (embedding surface, `crates/adapters`)

For embedders linking `crates/adapters` directly (the `IdentityProvider` trait
signature itself is unchanged):

- **`JwksCache::new`/`with_ttl` gained a required admitted-algorithms parameter**
  (e.g. pass the same constant your validator advertises). Constructing without
  it no longer compiles.
- **`JwksCache::get_keys` returns `Arc<VerificationKeySet>`**, not
  `serde_json::Value`. Use `get_key(kid)` for the resolve → one rate-limited
  forced refetch → re-resolve → fail-closed path both built-in providers use.
- **Key-selection behavior changed** to one shared constructor
  (`VerificationKeySet::from_jwks`): keys declaring an unknown algorithm (e.g.
  `RSA-OAEP`) are rejected instead of being inferred from their key type;
  alg-less RSA / EC P-256 / OKP Ed25519 signing keys are now accepted on Apple's
  path too; and a duplicate-`kid` JWKS whose eligible entry appears second now
  validates (two *eligible* entries under one `kid` remain an error).
- **Discovery endpoint origins**: each provider's discovery document may name
  only origins pinned at config load (`endpoint_origins`, plus the issuer's own).
  The check currently ships in warning mode — undeclared origins log a warning
  and are served — and rejecting them (`Warn` → `Enforce`) is a separate future
  release-owner decision after one release of that telemetry, not part of this
  version.
- Every outbound provider request goes through `ProviderTransport` (status read
  before body; bodies bounded at the shared 64 KiB ceiling); webhook delivery
  keeps its own operator-timeout client by design.

### Noop

Disables user sync. This is the default when `user_sync.enabled` is `false` or the section is omitted.
