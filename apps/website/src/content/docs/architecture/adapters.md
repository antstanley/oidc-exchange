---
title: "Storage Adapters"
description: "Available database, session, audit, and key management adapters."
---

Adapters connect oidc-exchange to storage, signing keys, audit logs, and user event services. All adapters are compiled into one binary. Configuration selects which ones the service uses.

## User and session storage

The `[repository]` section selects storage for user records and sessions. Sessions contain refresh token data. Three storage backends are available.

### DynamoDB

DynamoDB stores users and sessions in one table. It supports user lookups by internal ID or provider ID, and session lookups by refresh token hash.

```toml
[repository]
adapter = "dynamodb"

[repository.dynamodb]
table_name = "oidc-exchange"
region = "us-east-1"       # optional, uses SDK default if omitted
```

DynamoDB suits AWS deployments, including Lambda functions that do not maintain database connections. The table schema is defined in `schemas/dynamodb/table-design.json`.

### PostgreSQL

PostgreSQL uses [sqlx](https://github.com/launchbadge/sqlx) to share database connections across requests. Use it when you need a relational database or direct SQL queries on user data.

```toml
[repository]
adapter = "postgres"

[repository.postgres]
url = "postgres://user:pass@localhost:5432/oidc_exchange"
max_connections = 5
```

### SQLite

SQLite stores data in a local file using [sqlx](https://github.com/launchbadge/sqlx). It needs no external database service and suits development or single-server deployments.

```toml
[repository]
adapter = "sqlite"

[repository.sqlite]
path = "./data/oidc-exchange.db"
```

## Session-only storage

The optional `[session_repository]` section selects separate storage for sessions. User records remain in the `[repository]` backend.

When `[session_repository]` is configured:

- User operations (create, get, update, delete) use the `[repository]` backend.
- Session operations (store refresh token, lookup by hash, revoke) use the `[session_repository]` backend.

When `[session_repository]` is not configured, all operations use the `[repository]` backend.

### Valkey / Redis

This adapter stores sessions in memory through the [fred](https://github.com/aembke/fred.rs) client. It works with Redis, Valkey, and ElastiCache.

```toml
[session_repository]
adapter = "valkey"

[session_repository.valkey]
url = "redis://localhost:6379"
key_prefix = "oidc:"
```

### LMDB

Lightning Memory-Mapped Database (LMDB) stores sessions locally as key-value pairs. The adapter uses the [heed](https://github.com/meilisearch/heed) Rust library. It suits single-server deployments that need a separate session store without a network service.

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

The `[key_manager]` section controls how access tokens are signed. Access tokens use the JSON Web Token (JWT) format.

### Local key signing

The local adapter loads a private key from disk and signs tokens within the service. It supports only Ed25519 (EdDSA) keys. Other algorithms cause a startup error.

```toml
[key_manager]
adapter = "local"

[key_manager.local]
private_key_path = "./keys/ed25519.pem"
algorithm = "EdDSA"        # Ed25519; the only algorithm the local key manager accepts
kid = "key-1"
```

Generate a key:

```bash
# Ed25519
openssl genpkey -algorithm ed25519 -out keys/ed25519.pem
```

Local key management is suitable for development and single-server deployments. AWS Key Management Service (KMS) provides managed key protection and access control.

### AWS KMS

Sign tokens using an AWS KMS asymmetric key. Both RSA (RS256/RS384/RS512 and PS256/PS384/PS512) and ECDSA (ES256/ES384/ES512) signing algorithms are supported. Each signing operation calls KMS. The private key stays in KMS.

```toml
[key_manager]
adapter = "kms"

[key_manager.kms]
key_id = "arn:aws:kms:us-east-1:123456789:key/abcd-1234"
algorithm = "ES256"
kid = "prod-key-1"
```

KMS handles key rotation transparently. The AWS software development kit (SDK) obtains credentials from sources such as environment variables, instance profiles, or ECS task roles.

### Noop

The noop adapter returns an error for every signing operation. Use it for deployments with the `admin` role that do not issue access tokens.

```toml
[key_manager]
adapter = "noop"
```

## Audit logging

The `[audit]` section controls where compliance and security events are sent. Every token exchange, refresh, revocation, registration denial, and user lifecycle event generates an audit record.

### Noop

Events are not sent to any external system. When the audit provider is down or absent, events are always written to stdout (info and below) or stderr (error and above) as structured JSON. This fallback happens regardless of adapter.

```toml
[audit]
adapter = "noop"
blocking_threshold = "warning"
```

### Stdout/Stderr

This adapter writes audit events as structured JSON. Events at info severity or below go to stdout. Events at error severity or above go to stderr.

```toml
[audit]
adapter = "stdout"
blocking_threshold = "warning"
```

### SQS

This adapter sends audit events to Amazon Simple Queue Service (SQS). Firehose or Lambda can then forward them to storage or analytics services such as S3 or Iceberg.

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

- A failed `TokenExchange` audit (severity: notice) logs to stdout and the token exchange succeeds.
- A failed `RegistrationDenied` audit (severity: warning) causes the request to fail with a 500 error.

## Rate limiting

The `[rate_limit]` section bounds per-request and failed-authentication budgets. Two adapters are available.

### In-process fixed window

This adapter counts requests within fixed time windows. It limits the number of stored keys and removes expired entries. Each process has its own limits, so instances do not share request budgets.

```toml
[rate_limit]
enabled = true
store = "in_process"
window = "1m"
per_ip = 60
per_ip_failures = 10
per_subject = 10
per_provider = 600
max_concurrent_requests = 256
max_entries = 10000
```

### Noop

Every check returns `Allow`. Selected when `rate_limit.enabled = false`.

```toml
[rate_limit]
enabled = false
```

## User sync

The `[user_sync]` section enables outbound notifications when users are created, updated, or deleted.

### Webhook

The webhook adapter sends user events to an external URL with HTTP POST requests. It authenticates each request body with a hash-based message authentication code using SHA-256 (HMAC-SHA256).

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

The webhook sends this payload:

```json
{
  "event": "user.created",
  "timestamp": "2026-03-24T10:00:00Z",
  "data": { }
}
```

The event types are `user.created`, `user.updated`, and `user.deleted`. If user synchronization fails, the service logs a warning with `tracing::warn!`. The failure does not fail the original request.

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
therefore a retry of one delivery; treat it as such, not as an anomaly.

A conforming receiver **must**:

1. Verify the signature **before parsing the body**, which makes
   `X-Webhook-Timestamp` an authenticated value.
2. Reject deliveries whose `X-Webhook-Timestamp` is outside ±5 minutes of the
   receiver's clock. This bounds replay of a captured delivery to the tolerance
   window.
3. Deduplicate on `X-Webhook-Delivery-Id`, retaining seen ids for at least that
   ±5-minute window, at least as long as timestamps are trusted, so no expired
   delivery can be replayed past the dedup memory.
4. Treat any 2xx as success. 5xx and timeout responses are retried by the sender
   up to the configured `retries` count with exponential backoff; 4xx is not
   retried.

Worked receiver example (Node.js):

```js
import { createHmac, timingSafeEqual } from "node:crypto";

export function verifyDelivery(req, rawBody, secret, seenIds, now = Date.now()) {
  // 1. Signature first, over exactly the documented input, before any JSON.parse.
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
    return { ok: true, reason: "retry of a delivered id (acknowledged, not reprocessed)" };
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

Every existing receiver rejects every delivery until it is updated; there is no
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
  The check currently ships in warning mode (undeclared origins log a warning
  and are served), and rejecting them (`Warn` to `Enforce`) is a separate future
  release-owner decision after one release of that telemetry, not part of this
  version.
- Every outbound provider request goes through `ProviderTransport` (status read
  before body; bodies bounded at the shared 64 KiB ceiling); webhook delivery
  keeps its own operator-timeout client by design.

### Noop

The noop adapter disables user synchronization. It is the default when `user_sync.enabled` is `false` or the section is omitted.
