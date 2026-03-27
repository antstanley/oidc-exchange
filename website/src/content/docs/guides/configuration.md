---
title: "Configuration"
description: "Complete configuration reference for oidc-exchange."
---

oidc-exchange is configured entirely via TOML files and environment variables. A single configuration file controls every aspect of the service: server settings, token lifetimes, storage backends, identity providers, audit logging, and telemetry.

## Config loading order

Configuration is loaded and merged in the following order, with later sources overriding earlier ones:

1. **`config/default.toml`** --- baseline defaults shipped with the binary
2. **`config/{OIDC_EXCHANGE_ENV}.toml`** --- environment-specific overrides. The `OIDC_EXCHANGE_ENV` environment variable selects the file (e.g., `production`, `staging`, `local`). If unset, only `default.toml` is loaded.
3. **Environment variable overrides** --- structural overrides using double-underscore delimiters: `OIDC_EXCHANGE__{section}__{key}` (e.g., `OIDC_EXCHANGE__SERVER__PORT=9090`)
4. **`${VAR_NAME}` placeholder resolution** --- any value in the TOML containing `${VAR_NAME}` is resolved from the environment at load time

Secrets (client secrets, API keys, KMS ARNs) should always use `${VAR_NAME}` placeholders and be injected via environment variables. Never hardcode secrets in TOML files.

## Full annotated config example

The following shows every configuration section with all available options. In practice, most deployments only need a subset.

```toml
# ─── Server ───────────────────────────────────────────────────────
[server]
host = "0.0.0.0"                       # bind address
port = 8080                            # listen port
issuer = "https://auth.example.com"    # issuer URL for JWTs (iss claim)

# ─── Registration policy ──────────────────────────────────────────
[registration]
# "open" — any authenticated user gets a record created
# "existing_users_only" — user must already exist (created via /internal/users)
mode = "open"
# Optional — if set, only these email domains are allowed (applies in both modes)
# Exact match: "example.com"
# Wildcard: "*.acme.corp" (matches any subdomain depth)
# domain_allowlist = ["example.com", "*.acme.corp"]

# ─── Token settings ───────────────────────────────────────────────
[token]
access_token_ttl = "15m"               # short-lived JWT lifetime
refresh_token_ttl = "30d"              # long-lived refresh token lifetime
audience = "https://api.example.com"   # aud claim in access tokens

# Custom claims added to every access token JWT.
# Static values are used as-is.
# Template values reference the User model with {{ field }} syntax.
# The | default: filter provides a fallback if the field is missing.
# Reserved claims (sub, iss, aud, iat, exp) cannot be overridden.
[token.custom_claims]
org = "example"
role = "{{ user.metadata.role | default: 'user' }}"
tier = "{{ user.metadata.membership | default: 'free' }}"

# ─── Key management ───────────────────────────────────────────────
[key_manager]
adapter = "local"                      # "local" or "kms"

# Local key signing — load a PEM private key from disk
[key_manager.local]
private_key_path = "./keys/ed25519.pem"
algorithm = "EdDSA"                    # "EdDSA" (Ed25519) or "ES256" (P-256)
kid = "key-1"                          # key ID for JWT kid header

# AWS KMS — sign with a KMS asymmetric key
[key_manager.kms]
key_id = "arn:aws:kms:us-east-1:123456789:key/abcd-1234"
algorithm = "ECDSA_SHA_256"            # KMS signing algorithm (ECC_NIST_P256)
kid = "key-2024-01"

# ─── User and session storage ─────────────────────────────────────
[repository]
adapter = "dynamodb"                   # "dynamodb", "postgres", or "sqlite"

[repository.dynamodb]
table_name = "oidc-exchange"
region = "us-east-1"                   # optional, uses SDK default if omitted

[repository.postgres]
url = "postgres://user:pass@localhost:5432/oidc_exchange"
max_connections = 5

[repository.sqlite]
path = "./data/oidc-exchange.db"

# ─── Session-only storage (optional) ──────────────────────────────
# If set, session/refresh-token operations use this backend instead of
# the main repository. Useful for pairing a relational user store with
# a fast session store.
[session_repository]
adapter = "valkey"                     # "valkey" or "lmdb"

[session_repository.valkey]
url = "redis://localhost:6379"
key_prefix = "oidc:"

[session_repository.lmdb]
path = "./lmdb"
max_size_mb = 64

# ─── Audit logging ────────────────────────────────────────────────
[audit]
adapter = "noop"                       # "noop", "cloudtrail", or "sqs"
# Severity threshold for blocking. If the audit provider fails and the
# event's severity is at or above this threshold, the operation fails.
# Severities (RFC 5424): emergency, alert, critical, error, warning, notice, info, debug
blocking_threshold = "warning"

[audit.cloudtrail]
channel_arn = "arn:aws:cloudtrail:us-east-1:123456789:channel/my-channel"

# SQS adapter — send audit events to an SQS queue (e.g., for Firehose → S3/Iceberg pipeline)
[audit.sqs]
queue_url = "https://sqs.us-east-1.amazonaws.com/123456789/audit-queue"

# ─── User sync (webhook) ──────────────────────────────────────────
[user_sync]
enabled = false                        # set to true to enable
adapter = "webhook"                    # "webhook" or "noop"

[user_sync.webhook]
url = "https://internal-api.example.com/user-events"
secret = "${SYNC_WEBHOOK_SECRET}"      # HMAC-SHA256 signing secret
timeout = "5s"
retries = 2

# ─── Telemetry (OpenTelemetry) ────────────────────────────────────
[telemetry]
enabled = true
exporter = "otlp"                      # "otlp", "stdout", "xray", or "none"
endpoint = "http://localhost:4317"     # OTLP collector endpoint
service_name = "oidc-exchange"
sample_rate = 1.0                      # 0.0 to 1.0
protocol = "grpc"                      # "grpc" or "http"

# ─── Internal admin API ───────────────────────────────────────────
[internal_api]
auth_method = "shared_secret"
shared_secret = "${INTERNAL_API_SECRET}"

# ─── Identity providers ───────────────────────────────────────────
# Each [providers.<name>] block registers a provider. The name is used
# in POST /token requests as the "provider" field.

[providers.google]
adapter = "oidc"
issuer = "https://accounts.google.com"
client_id = "${GOOGLE_CLIENT_ID}"
client_secret = "${GOOGLE_CLIENT_SECRET}"
scopes = ["openid", "email", "profile"]

[providers.apple]
adapter = "apple"
client_id = "com.example.app"
team_id = "${APPLE_TEAM_ID}"
key_id = "${APPLE_KEY_ID}"
private_key_path = "/secrets/apple.p8"

[providers.atproto]
adapter = "atproto"
client_id = "https://example.com/oauth/client-metadata.json"
```

## Environment variable overrides

Any config value can be overridden at runtime using environment variables with double-underscore delimiters:

```
OIDC_EXCHANGE__{section}__{key}=value
```

Examples:

| Environment variable | Config path | Effect |
|---|---|---|
| `OIDC_EXCHANGE__SERVER__PORT=9090` | `server.port` | Change listen port |
| `OIDC_EXCHANGE__REGISTRATION__MODE=existing_users_only` | `registration.mode` | Restrict registration |
| `OIDC_EXCHANGE__TOKEN__ACCESS_TOKEN_TTL=5m` | `token.access_token_ttl` | Shorten token lifetime |
| `OIDC_EXCHANGE__TELEMETRY__ENABLED=true` | `telemetry.enabled` | Enable telemetry |

## Secret placeholder syntax

Values containing `${VAR_NAME}` are resolved from the environment at config load time. This allows secrets to be injected without appearing in TOML files:

```toml
[providers.google]
client_id = "${GOOGLE_CLIENT_ID}"
client_secret = "${GOOGLE_CLIENT_SECRET}"
```

At startup, if `GOOGLE_CLIENT_ID` is set to `123456.apps.googleusercontent.com`, the config value becomes that string. If the environment variable is not set, the service fails to start with a configuration error.

## Defaults

| Setting | Default |
|---|---|
| `server.host` | `0.0.0.0` |
| `server.port` | `8080` |
| `registration.mode` | `open` |
| `registration.domain_allowlist` | none (all domains allowed) |
| `token.access_token_ttl` | `15m` |
| `token.refresh_token_ttl` | `30d` |
| `telemetry.enabled` | `false` |
| `telemetry.exporter` | `none` |
| `telemetry.sample_rate` | `1.0` |
| `audit.adapter` | `noop` |
| `audit.blocking_threshold` | `warning` |
| `user_sync.enabled` | `false` |
| `internal_api` | disabled |
