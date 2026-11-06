---
title: "Configuration"
description: "Complete configuration reference for oidc-exchange."
---

Configure oidc-exchange with TOML files and environment variables. TOML is a text format for configuration settings. These settings control the server, token lifetimes, storage, identity providers, audit logging, and telemetry, which records service activity.

## Config loading order

Configuration is loaded and merged in the following order, with later sources overriding earlier ones:

1. Load baseline defaults from `config/default.toml`.
2. Load overrides from `config/{OIDC_EXCHANGE_ENV}.toml`. Set `OIDC_EXCHANGE_ENV` to select an environment such as `production`, `staging`, or `local`. If unset, the service loads only `default.toml`.
3. Apply environment variables named `OIDC_EXCHANGE__{section}__{key}`. Separate each part with two underscores. For example, `OIDC_EXCHANGE__SERVER__PORT=9090` sets the port.
4. Replace `${VAR_NAME}` placeholders with values from the environment. An unset variable causes a configuration error.
5. Validate the merged settings against the accepted types and values. Invalid security settings, including non-HTTPS issuer, provider, or webhook URLs, cause a startup error.

Use `${VAR_NAME}` placeholders for secrets such as client secrets and API keys. Supply their values through environment variables. Do not store secrets in TOML files.

## Full annotated config example

The following shows the configuration sections and the most common options. In practice, most deployments only need a subset.

```toml
# ─── Server ───────────────────────────────────────────────────────
[server]
host = "0.0.0.0"                       # bind address
port = 8080                            # listen port
issuer = "https://auth.example.com"    # issuer URL for JWTs (iss claim)
role = "exchange"                      # "exchange" (default), "admin", or "all" (see Upgrading below)
request_timeout = "30s"                # per-request timeout enforced by the server
max_request_body_bytes = 2097152       # max request body (2 MiB) buffered before processing
# base_path = "/prod"                  # optional path prefix stripped from request paths before routing
# Trust X-Forwarded-For only from these peer CIDRs. Empty means never trust it.
trusted_proxies = ["10.0.0.0/8"]
trusted_proxy_hops = 1                 # select this many entries from X-Forwarded-For's right side

# ─── Session location (opt-in) ────────────────────────────────────
# Name the request headers a trusted proxy sets; an empty name disables that field.
[session.location]
city_header = ""
region_header = ""
country_header = ""
ip_header = ""                         # client address, e.g. recorded as `reported`
trust_headers_from_any_peer = false    # see "Session location" below

# ─── Registration policy ──────────────────────────────────────────
[registration]
# "open": any authenticated user gets a record created
# "existing_users_only": user must already exist (created via /internal/users)
mode = "open"
# Optional. If set, only these email domains are allowed (applies in both modes)
# Exact match: "example.com"
# Wildcard: "*.acme.corp" (matches any subdomain depth)
# domain_allowlist = ["example.com", "*.acme.corp"]

# ─── Token settings ───────────────────────────────────────────────
[token]
access_token_ttl = "15m"               # short-lived JWT lifetime
refresh_token_ttl = "30d"              # long-lived refresh token lifetime
audience = "https://api.example.com"   # aud claim in access tokens
refresh_rotation = true                # rotate the refresh token on every redemption (default)
refresh_rotation_grace = "10s"         # window the just-superseded token still redeems (max 60s)
refresh_reuse_retention = "24h"        # how long a retired token is remembered for reuse detection

# Custom claims added to every access token JWT.
# Static values are used as-is.
# Template values reference the User model with {{ field }} syntax.
# The | default: filter provides a fallback if the field is missing.
# Reserved protocol claim names cannot be used as keys here or as per-user
# claims via the internal API; a configuration carrying one fails startup.
[token.custom_claims]
org = "example"
role = "{{ user.metadata.role | default: 'user' }}"
tier = "{{ user.metadata.membership | default: 'free' }}"

# ─── Grants ───────────────────────────────────────────────────────
# authorization_code and refresh_token are always served. The direct
# ID-token grant is opt-in and also gates the POST /nonce route.
[grants]
id_token = false                       # serve the direct ID-token assertion grant (default off)
nonce_ttl = "10m"                      # how long a minted nonce stays claimable
max_assertion_lifetime = "1h"          # ceiling on an accepted provider ID token's remaining life

# ─── Key management ───────────────────────────────────────────────
[key_manager]
adapter = "local"                      # "local" or "kms"

# Local key signing: load a PEM private key from disk
[key_manager.local]
private_key_path = "./keys/ed25519.pem"
algorithm = "EdDSA"                    # EdDSA only: the local adapter signs Ed25519 keys
kid = "key-1"                          # key ID for JWT kid header

# AWS KMS: sign with a KMS asymmetric key
[key_manager.kms]
key_id = "arn:aws:kms:us-east-1:123456789:key/abcd-1234"
algorithm = "ES256"                    # JWS signing algorithm (ECC_NIST_P256)
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
run_migrations = true                  # run schema migrations at startup (default true)

[repository.sqlite]
path = "./data/oidc-exchange.db"

# ─── Session-only storage (optional) ──────────────────────────────
# If set, session/refresh-token operations use this backend instead of
# the main repository. Useful for pairing a relational user store with
# a fast session store.
[session_repository]
adapter = "valkey"                     # "valkey" or "lmdb"
cleanup_interval = "1h"                # how often expired sessions/retirement records are swept

[session_repository.valkey]
url = "redis://localhost:6379"
key_prefix = "oidc:"

[session_repository.lmdb]
path = "./lmdb"
max_size_mb = 64

# ─── Audit logging ────────────────────────────────────────────────
[audit]
adapter = "stdout"                     # "noop", "stdout", "stderr", "auto", or "sqs" (default: stdout)
# Best-effort events below this severity are not dispatched. Shipped security events use
# the mandatory channel, which no threshold suppresses.
emit_threshold = "info"
# Best-effort sink-failure policy threshold (RFC 5424 severities).
blocking_threshold = "warning"
# Mandatory security-event sink failures: "observe" logs degradation; "enforce" fails the operation.
durability = "observe"

# ─── Public-route rate limiting ───────────────────────────────────
[rate_limit]
enabled = true
store = "in_process"                   # "in_process" or "none"
window = "1m"
per_ip = 60
per_ip_failures = 10                    # consumed only by authentication failures
per_subject = 10
per_provider = 600
max_concurrent_requests = 256
max_entries = 10000

# SQS adapter: send audit events to an SQS queue (e.g., for a Firehose to S3/Iceberg pipeline)
[audit.sqs]
queue_url = "https://sqs.us-east-1.amazonaws.com/123456789/audit-queue"
region = "us-east-1"                   # optional, uses SDK default if omitted

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
exporter = "otlp"                      # "otlp", "stdout", "xray", "prometheus", or "none"
endpoint = "http://localhost:4317"     # OTLP collector endpoint
service_name = "oidc-exchange"
sample_rate = 1.0                      # 0.0 to 1.0
protocol = "grpc"                      # "grpc" or "http"

# ─── Internal admin API ───────────────────────────────────────────
[internal_api]
enabled = true                         # serves /internal/* on the dedicated admin listener
host = "127.0.0.1"                     # admin listener bind address (loopback by default)
port = 8081
# Authentication mechanisms, tried in the order given.
# Values: "operator_token", "mtls", "shared_secret". The legacy singular
# `auth_method` key is still accepted and read as a one-element list.
auth_methods = ["operator_token"]
# Shared secret for the "shared_secret" compatibility mechanism. While that
# mechanism is enabled it must be at least 32 bytes, non-empty is not enough.
shared_secret = "${INTERNAL_API_SECRET}"

# "operator_token" mechanism: operator JWTs are verified against THIS
# service's own key manager ([key_manager]) and require a real (non-noop)
# adapter plus a non-empty [server].issuer.
token_audience = "internal"            # aud an operator token must carry; must differ from [token].audience
required_claim = "role"                # claim name a verified operator token must carry
required_value = "${OPERATOR_ROLE_VALUE}"  # value required_claim must carry

# Failed-authentication throttle, keyed by peer address on the admin listener:
max_auth_failures = 5                  # failed attempts one peer may spend...
auth_failure_window = "1m"             # ...per this window before lockout
auth_lockout = "5m"                    # how long a locked-out peer stays denied

# How long cached dashboard counts (active sessions) may be served before a
# re-scan; consumed by the DynamoDB adapter only. Valid range: 1s–3600s.
stats_cache_ttl = "60s"

# "mtls" mechanism: the client-certificate subject is asserted by the
# TLS-terminating proxy via this header. Trustworthy only while the admin
# listener is unreachable except through that proxy (which must also strip
# client-supplied copies of the header); a startup warning fires when this
# mechanism is enabled on a non-loopback listener.
[internal_api.mtls]
subject_header = "x-client-cert-subject"

# ─── Identity providers ───────────────────────────────────────────
# Each [providers.<name>] block registers a provider. The name is used
# in POST /token requests as the "provider" field.

[providers.google]
adapter = "oidc"
issuer = "https://accounts.google.com"
client_id = "${GOOGLE_CLIENT_ID}"
client_secret = "${GOOGLE_CLIENT_SECRET}"
scopes = ["openid", "email", "profile"]
# Extra origins Google's discovery document may name beyond accounts.google.com:
# token/revocation endpoints on oauth2.googleapis.com, JWKS on www.googleapis.com.
# Each entry is a bare https origin; defaults to empty (issuer's origin only).
endpoint_origins = ["https://oauth2.googleapis.com", "https://www.googleapis.com"]

[providers.apple]
adapter = "apple"
client_id = "com.example.app"
team_id = "${APPLE_TEAM_ID}"
key_id = "${APPLE_KEY_ID}"
private_key_path = "/secrets/apple.p8"

# atproto is a planned provider; see .specs/changes/2026-06-24-add_atproto_provider.md
```

## Environment variable overrides

Override a configuration value with an environment variable. Separate the prefix, section, and key with two underscores:

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

At startup, the service replaces `${VAR_NAME}` with the corresponding environment variable. This keeps secret values out of configuration files:

```toml
[providers.google]
client_id = "${GOOGLE_CLIENT_ID}"
client_secret = "${GOOGLE_CLIENT_SECRET}"
```

At startup, if `GOOGLE_CLIENT_ID` is set to `123456.apps.googleusercontent.com`, the config value becomes that string. If the environment variable is not set, the service fails to start with a configuration error.

Use `oidc-exchange config check path/to/config.toml` to run the same side-effect-free resolver used at startup. In this single-path form it merges the file with committed defaults but intentionally ignores environment variables and overlays, so pass a fully materialized deployment file. To check how the server would load configuration with the environment applied, use `oidc-exchange config check --dir <config-dir>` (the `default.toml` / `$OIDC_EXCHANGE_ENV.toml` layering plus `OIDC_EXCHANGE__*` overrides) or `oidc-exchange config check --file <path>` (a single file plus `OIDC_EXCHANGE__*` overrides, as used by the language bindings). It reports missing deployment inputs (such as unresolved placeholders) separately from invalid value domains, and redacts secrets in its rendered output.

## Defaults

| Setting | Default |
|---|---|
| `server.host` | `0.0.0.0` |
| `server.port` | `8080` |
| `server.role` | `exchange` |
| `server.issuer`, `token.audience` | `https://auth.example.com` / `https://api.example.com` (deployment placeholders; replace before production) |
| `registration.mode` | `open` |
| `registration.domain_allowlist` | none (all domains allowed) |
| `token.access_token_ttl` | `15m` |
| `token.refresh_token_ttl` | `30d` |
| `token.refresh_rotation` / `refresh_rotation_grace` / `refresh_reuse_retention` | `true` / `10s` / `24h` |
| `grants.id_token` / `nonce_ttl` / `max_assertion_lifetime` | `false` / `10m` / `1h` |
| `server.request_timeout` / `max_request_body_bytes` | `30s` / `2097152` (2 MiB) |
| `session_repository.cleanup_interval` | `1h` |
| `telemetry.enabled` | `false` |
| `telemetry.exporter` | `none` |
| `telemetry.sample_rate` | `1.0` |
| `audit.adapter` | `stdout` |
| `audit.durability` | `observe` |
| `audit.emit_threshold` | `info` |
| `rate_limit.enabled` / `store` / `window` | `true` / `in_process` / `1m` |
| `rate_limit.per_ip` / `per_ip_failures` / `per_subject` / `per_provider` | `60` / `10` / `10` / `600` |
| `server.trusted_proxies` / `trusted_proxy_hops` | `[]` / `1` |
| `session.location.*_header` / `trust_headers_from_any_peer` | `""` / `false` |
| `audit.blocking_threshold` | `warning` |
| `providers.<name>.endpoint_origins` | none; the provider is pinned to its issuer's origin (plus the origins of explicitly configured endpoints) |
| `user_sync.enabled` | `false` |
| `internal_api.enabled` | `false` |
| `internal_api.host` / `.port` | `127.0.0.1:8081` |
| `internal_api.auth_methods` | none (empty; must be set explicitly when the internal API is served, or startup fails) |
| `internal_api.token_audience` | `"internal"` |
| `internal_api.required_claim` / `.required_value` | `"role"` / `"admin"` |
| `internal_api.mtls.subject_header` | `"x-client-cert-subject"` |
| `internal_api.max_auth_failures` | `5` |
| `internal_api.auth_failure_window` | `"1m"` |
| `internal_api.auth_lockout` | `"5m"` |
| `internal_api.stats_cache_ttl` | `"60s"` |

## Upgrading: `server.role` now defaults to `exchange`

Earlier releases defaulted `server.role` to `all`: a deployment that only set
`internal_api.enabled = true` got the internal admin API served on the same
process as the public `/token` endpoint without ever naming that decision. The
default is now `exchange`, which serves only the public exchange plane;
admin reachability must be a deliberate deployment decision, visible in
configuration.

If an installation relied on the implicit `all`, set the role explicitly when
upgrading:

```toml
[server]
role = "all"     # public exchange plane + internal admin API on this process
```

Prefer splitting the planes when you do: keep `role = "exchange"` (or omit the
key) on internet-facing processes, and run a separate process with
`role = "admin"` and `internal_api.enabled = true` for the admin API, reachable
only from your operator network. An `exchange`-role process never mounts
`/internal/*`, so a forgotten `internal_api.enabled = true` there fails closed
instead of publishing the privilege-assignment primitive.

## Trusted proxies and rate limiting

The server uses the connection peer as the client address by default. It reads
`X-Forwarded-For` only when that peer belongs to `server.trusted_proxies`; it then selects
`trusted_proxy_hops` entries from the **right** of the comma-separated chain. This makes the
address `forwarded`; direct peers are `peer`. A forwarding header from any other peer is
client-asserted and is never used as a rate-limit key or authorization input.

## Session location

`GET /internal/users/{id}/sessions` lists a user's live sessions (one per refresh-token family,
newest first, at most 50) with an opaque stable `id` and an optional `location`
(`city`, `region`, `country`). The service records whatever location its caller supplies; the
HTTP server fills it from the request headers named under `[session.location]`. Nothing is
read by default, and no provider name appears in the code: header names are configuration.
Values are percent-decoded, stripped of control characters, trimmed, and capped at 80
characters per part.

Headers are honoured only when the request's peer is in `server.trusted_proxies`. When
`ip_header` is also named and holds a valid address (a trailing `:port` and IPv6 brackets are
stripped), that address is recorded as `reported` and, because the peer is trusted, may key
the per-IP throttle; it wins over `X-Forwarded-For`.

Platforms whose peer address is not a stable, listable CIDR (Lambda behind an API gateway)
can set `trust_headers_from_any_peer = true`. This states a fact the service cannot verify:
the origin is unreachable except through the proxy that sets these headers, which must strip
or overwrite any client-supplied copies. An address accepted this way is recorded as
`asserted` and is never a rate-limit key. The setting is rejected at load when no header is
named.

The shipped in-process limiter is per process. Use an API gateway, WAF, or other edge control
for a global limit across replicas or Lambda execution environments.
