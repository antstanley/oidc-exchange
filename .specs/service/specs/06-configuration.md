# Configuration

**Status:** Implemented · **Date:** 2026-08-17 · **Owner:** Ant Stanley · **Scope:** crates/core/src/config.rs, config/

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
trusted_proxies = []
trusted_proxy_hops = 1

[registration]
mode = "open"

[token]
access_token_ttl = "15m"
refresh_token_ttl = "30d"

[audit]
adapter = "stdout"
blocking_threshold = "warning"
emit_threshold = "info"
durability = "observe"

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

[telemetry]
enabled = false
exporter = "none"
```

The default is deliberately minimal — no providers or key manager — but it is no longer
silent: once a deployment supplies the required service configuration, audit events go to
stdout. A real deployment overlays a key manager, a repository, and at least one provider.

## Sections

### `[server]`
`host` (`0.0.0.0`), `port` (`8080`), `issuer` (the `iss` claim / discovery issuer, default
empty), `role` (`all` | `exchange` | `admin`, default `all`), `request_timeout` (humantime
duration string like the token TTLs, default `"30s"`), `trusted_proxies` (CIDR list, default
empty), and `trusted_proxy_hops` (default `1`). The hop is counted from the right of
`X-Forwarded-For` only when the observed peer is in `trusted_proxies`; with the default empty
list, forwarding headers are not trusted.

### `[registration]`
`mode` (`open` | `existing_users_only`, default `open`), optional `domain_allowlist`
(`Vec<String>`; exact or `*.domain` wildcard).

### `[token]`
`access_token_ttl` (`"15m"`), `refresh_token_ttl` (`"30d"`), optional `audience`, optional
`custom_claims` (`HashMap<String,String>` of claim templates, see
[03-service-flows.md](03-service-flows.md)).

### `[audit]`
`adapter` (`noop` | `stdout` | `sqs`, default `stdout`), `blocking_threshold` (syslog
severity name, default `warning`), `emit_threshold` (default `info`) for best-effort events,
and `durability` (`observe` | `enforce`, default `observe`) for mandatory security-event write
failures; optional `[audit.sqs] { queue_url, region }`.

### `[rate_limit]`
`enabled` (default `true`), `store` (`in_process` | `none`, default `in_process`), `window`
(default `"1m"`), per-window budgets `per_ip` (60), `per_ip_failures` (10), `per_subject`
(10), `per_provider` (600), `max_concurrent_requests` (256), and `max_entries` (10000).
Budgets are per key and per process; zero disables a scope. `per_ip_failures` is consumed only
for authentication failures, preserving the normal IP allowance for legitimate requests.

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
max_size_mb? }`. Absent → sessions live in the `[repository]` store.

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
| `server.trusted_proxies` / `trusted_proxy_hops` | `[]` / `1` |
| `registration.mode` | `open` |
| `token.access_token_ttl` / `refresh_token_ttl` | `15m` / `30d` |
| `audit.adapter` / `blocking_threshold` / `emit_threshold` / `durability` | `stdout` / `warning` / `info` / `observe` |
| `rate_limit.enabled` / `store` / `window` / `max_concurrent_requests` | `true` / `in_process` / `"1m"` / `256` |
| `rate_limit.per_ip` / `per_ip_failures` / `per_subject` / `per_provider` | `60` / `10` / `10` / `600` |
| `telemetry.enabled` / `exporter` / `sample_rate` | `false` / `none` / `1.0` |
| `user_sync.enabled`, `internal_api.enabled` | `false`, `false` |

## Assumptions and open questions

### Assumptions

- Secrets are supplied through the environment and referenced via `${VAR}`; secrets are never
  committed to a TOML file.
- Config is read once at startup; changing it requires a restart.
- `audit.adapter` must be a known non-empty adapter, `audit.durability` must be `observe` or
  `enforce`, trusted proxies must be CIDRs, `trusted_proxy_hops` must be 1–16, and the
  rate-limit window, store, budgets, entry bound, and concurrency bound are validated at load
  time. `rate_limit.enabled = true` requires `store = "in_process"`.
- Where a rate limit must hold globally (Lambda or horizontally scaled servers), an edge
  gateway/WAF provides it; the in-process limiter is a per-process backstop.

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
