# Configuration

**Status:** Implemented · **Date:** 2026-07-02 · **Owner:** Ant Stanley · **Scope:** crates/core/src/config.rs, config/

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
empty), `role` (`all` | `exchange` | `admin`, default `all`), `request_timeout` (humantime
duration string like the token TTLs, default `"30s"`) — the per-request timeout the
server's timeout layer enforces.

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
`extra: HashMap<String, toml::Value>`. `endpoint_origins` is an optional array of `scheme
"://" host [":" port]` origins that a discovery document is permitted to name in addition to
the issuer's own origin and the origins of any explicitly configured endpoint; each entry
must parse as an `https` origin with no path, query, or fragment. It defaults to empty,
which pins a provider to its issuer's origin. While the endpoint-origin check runs in its
shipped warning mode, an undeclared origin logs a structured warning naming the endpoint
and the permitted set and the deployment is served unchanged; rejecting undeclared origins
is a separate future release-owner decision after one release of that telemetry. See
[05-provider-system.md](05-provider-system.md).

## Defaults summary

| Setting | Default |
|---|---|
| `server.host` / `port` / `role` / `request_timeout` | `0.0.0.0` / `8080` / `all` / `"30s"` |
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
- *Endpoint origins are declared, not derived.* **`endpoint_origins` lists the extra origins
  a provider's discovery document may name.** Deriving the permitted set from the issuer
  alone would reject Google, whose `token_endpoint`, `jwks_uri`, and `revocation_endpoint`
  are on two origins that are neither the issuer nor each other; deriving it from the
  discovery document is what the constraint exists to prevent. Declaring it makes the
  trusted set reviewable in the same file that names the provider.

### Open questions

- None.
