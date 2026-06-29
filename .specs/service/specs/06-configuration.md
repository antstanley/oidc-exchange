# Configuration

**Status:** Implemented · **Date:** 2026-06-29 · **Owner:** Ant Stanley · **Scope:** crates/core/src/config.rs, config/

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

[audit]
adapter = "noop"
blocking_threshold = "warning"

[telemetry]
enabled = false
exporter = "none"
```

The default is deliberately minimal — runnable but inert (noop audit, no providers, no key
manager). A real deployment overlays a key manager, a repository, and at least one provider.

## Sections

### `[server]`
`host` (`0.0.0.0`), `port` (`8080`), `issuer` (the `iss` claim / discovery issuer, default
empty), `role` (`all` | `exchange` | `admin`, default `all`).

### `[registration]`
`mode` (`open` | `existing_users_only`, default `open`), optional `domain_allowlist`
(`Vec<String>`; exact or `*.domain` wildcard).

### `[token]`
`access_token_ttl` (`"15m"`), `refresh_token_ttl` (`"30d"`), optional `audience`, optional
`custom_claims` (`HashMap<String,String>` of claim templates, see
[03-service-flows.md](03-service-flows.md)).

### `[audit]`
`adapter` (`noop` | `stdout` | `sqs`, default `noop`), `blocking_threshold` (syslog severity
name, default `warning`), optional `[audit.sqs] { queue_url, region }`.

### `[key_manager]`
`adapter` (`local` | `kms`), with `[key_manager.local] { private_key_path, algorithm, kid }`
or `[key_manager.kms] { key_id, algorithm, kid }`. Skipped (noop) in the `admin` role.

### `[repository]` (users + sessions)
`adapter` (`dynamodb` | `postgres` | `sqlite`), with one of
`[repository.dynamodb] { table_name, region? }`,
`[repository.postgres] { url, max_connections? }`,
`[repository.sqlite] { path }`.

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
| `server.host` / `port` / `role` | `0.0.0.0` / `8080` / `all` |
| `registration.mode` | `open` |
| `token.access_token_ttl` / `refresh_token_ttl` | `15m` / `30d` |
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
