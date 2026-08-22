# Configuration

**Status:** Implemented · **Date:** 2026-08-22 · **Owner:** Ant Stanley · **Scope:** crates/core/src/config.rs, config/

One TOML file drives the whole service. `AppConfig` (and its nested structs) in
`crates/core/src/config.rs` deserializes it; every section uses `#[serde(default)]`, so any
omitted section falls back to its defaults.

## Loading order (`bootstrap::load_config`)

1. `config/default.toml` — compiled-in defaults (committed; see below).
2. `config/{OIDC_EXCHANGE_ENV}.toml` — overlay when `OIDC_EXCHANGE_ENV` is set (e.g.
   `production`, `sqlite-only`); examples ship several named environments.
3. `OIDC_EXCHANGE__{section}__{key}` environment variables — structural overrides.
4. `${VAR_NAME}` placeholders anywhere in the merged config are resolved from the environment
   (used for secrets and per-deployment values).

## Committed default (`config/default.toml`)

```toml
[server]
host = "0.0.0.0"
port = 8080

[registration]
mode = "open"

[token]
access_token_ttl = "15m"
refresh_token_ttl = "30d"
refresh_rotation = true
refresh_rotation_grace = "10s"
refresh_reuse_retention = "24h"

[session_repository]
cleanup_interval = "1h"

[audit]
adapter = "noop"
blocking_threshold = "warning"

[telemetry]
enabled = false
exporter = "none"
```

The default is deliberately minimal — runnable but inert (noop audit, no providers, no key
manager). A real deployment overlays a key manager, a repository, and at least one provider.

## Validation at load

`AppConfig::validate` runs after deserialization and placeholder resolution, at the end of
every production entry point (`load_config`, `parse_config`), so malformed config fails
startup closed rather than being absorbed and discovered per-request:

- `server.role` is one of `all` / `exchange` / `admin`.
- `server.request_timeout`, `token.access_token_ttl`, `token.refresh_token_ttl`,
  `token.refresh_rotation_grace`, `token.refresh_reuse_retention`, and
  `session_repository.cleanup_interval` must parse as `<integer><s|m|h|d>` without
  overflow; the parsed values are reused at request time, which therefore cannot fail.
  `refresh_rotation_grace`, `refresh_reuse_retention`, and `cleanup_interval` must
  additionally be strictly positive, and `refresh_rotation_grace` is capped at `60s`.
- Every `registration.domain_allowlist` entry is an exact domain or a `*.`-prefixed wildcard.
- When the internal API will be served (`server.role` is `admin` or `all`, and
  `internal_api.enabled == true`), `internal_api.shared_secret` is present and non-empty.

## Sections

### `[server]`
`host` (`0.0.0.0`), `port` (`8080`), `issuer` (the `iss` claim / discovery issuer, default
empty), `role` (`all` | `exchange` | `admin`, default `all`), `request_timeout` (humantime
duration string like the token TTLs, default `"30s"`) — the per-request timeout the
server's timeout layer enforces.

### `[registration]`
`mode` (`open` | `existing_users_only`, default `open`), optional `domain_allowlist`
(`Vec<String>`; exact or `*.domain` wildcard).

### `[token]`
`access_token_ttl` (`"15m"`), `refresh_token_ttl` (`"30d"`), optional `audience`, optional
`custom_claims` (`HashMap<String,String>` of claim templates, see
[03-service-flows.md](03-service-flows.md)), `refresh_rotation` (bool, default `true`),
`refresh_rotation_grace` (duration string, default `"10s"`) and `refresh_reuse_retention`
(duration string, default `"24h"`).

`refresh_rotation_grace` is the window in which the immediately-preceding generation is
still redeemable; config loading rejects a value above `60s`, because the window is a
deliberate weakening and an unbounded one is indistinguishable from no rotation.
`refresh_reuse_retention` is how long a retired generation is remembered so its
re-presentation raises an alarm; it is capped per record at the family's own `expires_at`.

### `[audit]`
`adapter` (`noop` | `stdout` | `sqs`, default `noop`), `blocking_threshold` (syslog severity
name, default `warning`), optional `[audit.sqs] { queue_url, region }`.

### `[key_manager]`
`adapter` (`local` | `kms`), with `[key_manager.local] { private_key_path, algorithm, kid }`
or `[key_manager.kms] { key_id, algorithm, kid }`. Skipped (noop) in the `admin` role.

### `[repository]` (users + sessions)
`adapter` (`dynamodb` | `postgres` | `sqlite`), with one of
`[repository.dynamodb] { table_name, region? }`,
`[repository.postgres] { url, max_connections?, run_migrations? }`,
`[repository.sqlite] { path }`. `run_migrations` defaults to `true`; set it to `false` for
locked-down databases where the app role has no DDL rights and migrations are applied
out-of-band.

### `[session_repository]` (optional, sessions only)
When present, overrides where sessions are stored: `adapter` (`valkey` | `lmdb`) with
`[session_repository.valkey] { url, key_prefix? }` or `[session_repository.lmdb] { path,
max_size_mb? }`. Absent → sessions live in the `[repository]` store. `cleanup_interval`
(duration string, default `"1h"`) — how often the long-lived runtimes run
`cleanup_expired_sessions` ([04-http-api.md](04-http-api.md) → Bootstrap). The sweep covers
`sessions` and `retired_refresh_tokens` alike; on the natively-expiring stores (DynamoDB
TTL, Valkey key expiry) it is a cheap backstop for whatever native expiry has not yet
reaped.

### `[user_sync]`
`enabled` (bool), `adapter` (`webhook`), `[user_sync.webhook] { url, secret, timeout?,
retries? }`. The `secret` is redacted in `Debug`.

### `[telemetry]`
`enabled` (false), `exporter` (`none` | `stdout` | `otlp` | `xray`), optional `endpoint`,
`service_name`, `sample_rate` (default 1.0), `protocol`.

### `[internal_api]`
`enabled` (false), `auth_method` (`shared_secret`), `shared_secret` (redacted in `Debug`).

### `[providers.<name>]`
`adapter` (`oidc` | `apple`) plus adapter-specific fields captured via a flattened
`extra: HashMap<String, toml::Value>`. See [05-provider-system.md](05-provider-system.md).

## Defaults summary

| Setting | Default |
|---|---|
| `server.host` / `port` / `role` / `request_timeout` | `0.0.0.0` / `8080` / `all` / `"30s"` |
| `registration.mode` | `open` |
| `token.access_token_ttl` / `refresh_token_ttl` | `15m` / `30d` |
| `token.refresh_rotation` / `refresh_rotation_grace` / `refresh_reuse_retention` | `true` / `"10s"` / `"24h"` |
| `session_repository.cleanup_interval` | `"1h"` |
| `audit.adapter` / `blocking_threshold` | `noop` / `warning` |
| `telemetry.enabled` / `exporter` / `sample_rate` | `false` / `none` / `1.0` |
| `user_sync.enabled`, `internal_api.enabled` | `false`, `false` |

## Assumptions and open questions

### Assumptions

- Secrets are supplied through the environment and referenced via `${VAR}`; secrets are never
  committed to a TOML file.
- Config is read once at startup; changing it requires a restart.

### Decisions

- *`serde(default)` everywhere.* **Every config section has defaults.** A minimal TOML boots,
  and adding a field never breaks deserialization of existing files.
- *Secrets redacted in Debug.* **`WebhookConfig.secret` and `InternalApiConfig.shared_secret`
  have custom `Debug`.** Prevents secret leakage through log lines that dump config.
- *Separate session repository section.* **`[session_repository]` is optional and overrides
  only session storage.** Enables split topologies (SQL users + Valkey/LMDB sessions) without
  duplicating the user-store config.

### Open questions

- None.
