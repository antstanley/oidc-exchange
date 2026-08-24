# Configuration

**Status:** Implemented · **Date:** 2026-08-05 · **Owner:** Ant Stanley · **Scope:** crates/core/src/config.rs, config/

One TOML file drives the whole service. `AppConfig` (and its nested structs) in
`crates/core/src/config.rs` deserializes it; every section uses `#[serde(default)]`, so any
omitted section falls back to its defaults.

## Loading order (`bootstrap::load_config`)

1. `config/default.toml` — compiled-in defaults (committed; see below).
2. `config/{OIDC_EXCHANGE_ENV}.toml` — overlay when `OIDC_EXCHANGE_ENV` is set (e.g.
   `production`, `sqlite-only`); examples ship several named environments.
3. `OIDC_EXCHANGE__{section}__{key}` environment overrides apply on top of the merged TOML
   and reach every config path, including map-valued sections —
   `OIDC_EXCHANGE__PROVIDERS__GOOGLE__CLIENT_ID` sets `providers.google.client_id`. A double
   underscore separates path segments and each segment is lowercased; a single underscore
   stays inside its segment (`…__MY_IDP__…` targets `providers.my_idp`), so keys whose names
   themselves contain `__` cannot be addressed from the environment.
4. `${VAR_NAME}` placeholders anywhere in the merged config are resolved from the environment
   (used for secrets and per-deployment values). A placeholder that names an unset variable is
   a startup error — a secret never silently degrades to its literal placeholder text. `$${`
   escapes to a literal `${` and is never resolved.

## Validation at load

After merging and placeholder resolution, `load_config` validates the result and refuses to
start on failure (`ConfigError`):

- `server.role` must be one of `all` | `exchange` | `admin`.
- `server.request_timeout`, `token.access_token_ttl`, and `token.refresh_token_ttl` must parse
  as `<integer><s|m|h|d>` without overflow; the parsed values are reused at request time, which
  therefore cannot fail.
- Each `registration.domain_allowlist` entry must be an exact domain (`example.com`) or a
  `*.`-prefixed wildcard (`*.example.com`). Bare `*` and dotless prefixes (`*example.com`)
  are rejected.
- When the internal API will be served (`role` is `admin` or `all` and
  `internal_api.enabled = true`), `internal_api.shared_secret` must be present and non-empty.

The same validation runs for config supplied as a string through the FFI bindings
(`bootstrap::parse_config`).

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
server's timeout layer enforces — and `base_path` (optional, default unset — a leading prefix
such as `/prod` stripped from incoming request paths before routing; honored in both Lambda and
server mode, though it exists chiefly for API Gateway stages and mount prefixes).

### `[registration]`
`mode` (`open` | `existing_users_only`, default `open`), optional `domain_allowlist`
(`Vec<String>`; exact or `*.domain` wildcard).

### `[token]`
`access_token_ttl` (`"15m"`), `refresh_token_ttl` (`"30d"`), optional `audience`, optional
`custom_claims` (`HashMap<String,String>` of claim templates, see
[03-service-flows.md](03-service-flows.md)).

### `[audit]`
`adapter` (`noop` | `stdout` | `sqs`, default `noop`), `blocking_threshold` (syslog severity
name, default `warning`), `emit_threshold` (syslog severity name, default `info`)
— events with a severity strictly less severe than the threshold are not emitted at all,
independently of the blocking decision — optional `[audit.sqs] { queue_url, region }`.

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
`enabled` (false — internal routes are not mounted unless true, regardless of `server.role`;
a `role = "admin"` instance with the flag off serves only `/health`), `auth_method`
(`shared_secret`), `shared_secret` (redacted in `Debug`; must be non-empty when the internal
API is served).

### `[providers.<name>]`
`adapter` (`oidc` | `apple`) plus adapter-specific fields captured via a flattened
`extra: HashMap<String, toml::Value>`. See [05-provider-system.md](05-provider-system.md).

## Defaults summary

| Setting | Default |
|---|---|
| `server.host` / `port` / `role` / `request_timeout` | `0.0.0.0` / `8080` / `all` / `"30s"` |
| `registration.mode` | `open` |
| `token.access_token_ttl` / `refresh_token_ttl` | `15m` / `30d` |
| `audit.adapter` / `blocking_threshold` / `emit_threshold` | `noop` / `warning` / `info` |
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
