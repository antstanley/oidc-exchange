# Configuration

**Status:** Implemented · **Date:** 2026-08-16 · **Owner:** Ant Stanley · **Scope:** crates/core/src/config.rs, crates/server/src/bootstrap.rs, config/

One TOML file drives the whole service. `RawConfig` mirrors the merged TOML input; `AppConfig`
is the resolved configuration held by the service. Every entry point that constructs a running
service uses the same resolver, so a configuration that cannot be validated never reaches a
request path.

## Loading order (`bootstrap::load_config`)

1. `config/default.toml` — compiled-in defaults (committed; see below).
2. `config/{OIDC_EXCHANGE_ENV}.toml` — overlay when `OIDC_EXCHANGE_ENV` is set (for example,
   `production` or `sqlite-only`); examples ship several named environments.
3. `OIDC_EXCHANGE__{section}__{key}` environment variables — structural overrides.
4. `${VAR_NAME}` placeholders anywhere in the merged config are resolved from the environment;
   an unset variable is a config error (fail closed), and `$${` escapes a literal `${`.
5. `AppConfig::resolve` narrows the merged tree into the typed configuration the service runs
   on, rejecting any value outside its domain (see *Validation at load*). The server binary,
   Lambda handler, FFI `new`/`from_file` constructors, and `oidc-exchange config check <path>`
   all use this one resolver.

## Validation at load

Configuration is parsed in two stages. `RawConfig` is only an intermediate; `AppConfig` is what
the service holds, and its security-relevant fields are enums, newtypes, and constrained URLs
rather than arbitrary strings. `AppConfig::resolve(raw, env)` performs placeholder substitution,
applies environment overrides, and constructs the narrowed types, returning a `ConfigError`
naming the offending field. A value that cannot be narrowed aborts startup; there is no
permissive fallback and no per-request re-parse.

The closed domains:

| Field | Type | Domain |
|---|---|---|
| `server.role` | `ServerRole` | `all` \| `exchange` \| `admin` |
| `server.issuer` | `HttpsUrl` | required, non-empty, absolute `https` URL |
| `registration.mode` | `RegistrationMode` | `open` \| `existing_users_only` |
| `registration.domain_allowlist[]` | `AsciiDomainPattern` | exact domain or `*.`-prefixed wildcard; non-ASCII rejected |
| `token.audience` | `NonEmptyString` | required, non-empty |
| `token.access_token_ttl`, `token.refresh_token_ttl`, `server.request_timeout` | `Duration` | `<integer><s\|m\|h\|d>`, no overflow |
| `key_manager.local.algorithm` | `SigningAlgorithm` | `EdDSA` — the only algorithm the local adapter can produce |
| `key_manager.kms.algorithm` | `SigningAlgorithm` | `RS256` \| `RS384` \| `RS512` \| `PS256` \| `PS384` \| `PS512` \| `ES256` \| `ES384` \| `ES512` (JWS names, RFC 7518 §3.1 — not AWS `SigningAlgorithmSpec` names) |
| `audit.adapter` | `AuditAdapter` | `noop` \| `stdout` \| `stderr` \| `auto` \| `sqs` |
| `audit.blocking_threshold`, `audit.emit_threshold` | `AuditSeverity` | syslog severity name |
| `telemetry.exporter` | `TelemetryExporter` | `none` \| `stdout` \| `otlp` \| `xray` |
| `internal_api.auth_method` | `InternalAuthMethod` | `shared_secret` |
| `user_sync.webhook.url` | `HttpsUrl` | `https` only |
| `providers.<name>.{issuer,jwks_uri,token_endpoint,revocation_endpoint}` | `HttpsUrl` | `https` only |
| `providers.<name>.adapter` | `ProviderAdapter` | `oidc` \| `apple` |

Two cross-field checks run in the same pass:

- **Internal-API secret.** When the internal API will be served (`server.role` is `admin` or
  `all` and `internal_api.enabled = true`), `internal_api.shared_secret` must be present and
  non-empty.
- **Algorithm truthfulness.** The key manager reports the algorithm derived from the key
  material it loaded; resolution compares the operator's declared `algorithm` against it and
  fails when they disagree. The configured value is an assertion to verify, never metadata to
  republish.

`oidc-exchange config check <path>` runs the same side-effect-free resolver with no side
effects and prints the resolved configuration with secrets redacted. It merges the supplied file
with committed defaults but intentionally does not read environment variables, overlays, or
working-directory configuration. Use a fully materialized deployment file to validate it before
starting the service. Unresolved placeholders are reported as missing deployment inputs; invalid URL,
enum, duration, or algorithm values are configuration errors rather than values a fixture or
fallback can normalize.

## Committed default (`config/default.toml`)

```toml
[server]
host = "0.0.0.0"
port = 8080
issuer = "https://auth.example.com"

[registration]
mode = "open"

[token]
access_token_ttl = "15m"
refresh_token_ttl = "30d"
audience = "https://api.example.com"

[audit]
adapter = "noop"
blocking_threshold = "warning"

[telemetry]
enabled = false
exporter = "none"
```

The default is deliberately minimal — no key manager, no repository, no providers — but its
HTTPS issuer and audience are deliberately documentation placeholders, not production identity
values. A deployment must replace them with its own non-empty values before it can issue tokens
for its namespace. A service that would sign tokens carrying `iss: ""` and `aud: ""` is not
usefully runnable, and empty values are not representable in `AppConfig`.

## Sections

### `[server]`

`host` (`0.0.0.0`), `port` (`8080`), `issuer` (**required** — the `iss` claim and discovery
issuer; an absolute `https` URL, no default), `role` (`all` | `exchange` | `admin`, default
`all`), `request_timeout` (humantime duration string like the token TTLs, default `"30s"`) —
the per-request timeout the server's timeout layer enforces — and `base_path` (optional, default
unset — a leading prefix such as `/prod` stripped from incoming request paths before routing;
honored in both Lambda and server mode).

### `[registration]`

`mode` (`open` | `existing_users_only`, default `open`) is a closed domain: an unrecognised
value is a config-load error, never a silent selection of `open`. Optional `domain_allowlist`
contains exact or `*.domain` wildcard entries; entries are ASCII only.

### `[token]`

`access_token_ttl` (`"15m"`), `refresh_token_ttl` (`"30d"`), `audience` (**required**,
non-empty — the `aud` claim of every issued access token), and optional `custom_claims`
(`HashMap<String, String>` of claim templates; see [03-service-flows.md](03-service-flows.md)).

### `[audit]`

`adapter` (`noop` | `stdout` | `stderr` | `auto` | `sqs`, default `noop`),
`blocking_threshold` (syslog severity name, default `warning`), `emit_threshold` (syslog
severity name, default `info`), and optional `[audit.sqs] { queue_url, region }`.

### `[key_manager]`

`adapter` (`local` | `kms`), with `[key_manager.local] { private_key_path, algorithm, kid }`
or `[key_manager.kms] { key_id, algorithm, kid }`. `algorithm` is a JWS `alg` name (RFC 7518
§3.1), validated at load against the algorithms the selected adapter can actually produce —
`EdDSA` for `local`, and `RS`/`PS`/`ES` 256/384/512 for `kms`. AWS `SigningAlgorithmSpec` names
such as `ECDSA_SHA_256` are not accepted. Skipped (noop) in the `admin` role.

### `[repository]` (users + sessions)

`adapter` (`dynamodb` | `postgres` | `sqlite`), with one of
`[repository.dynamodb] { table_name, region? }`,
`[repository.postgres] { url, max_connections?, run_migrations? }`, or
`[repository.sqlite] { path }`. `run_migrations` defaults to `true`; set it to `false` for
locked-down databases where migrations are applied out-of-band.

### `[session_repository]` (optional, sessions only)

When present, overrides where sessions are stored: `adapter` (`valkey` | `lmdb`) with
`[session_repository.valkey] { url, key_prefix? }` or
`[session_repository.lmdb] { path, max_size_mb? }`. Absent means sessions live in the
`[repository]` store.

### `[user_sync]`

`enabled` (bool), `adapter` (`webhook`), and `[user_sync.webhook] { url, secret, timeout?,
retries? }`. `url` must be `https` because the payload carries the full user record. The
`secret` is redacted in `Debug`.

### `[telemetry]`

`enabled` (false), `exporter` (`none` | `stdout` | `otlp` | `xray`), optional `endpoint`,
`service_name`, `sample_rate` (default 1.0), and `protocol`.

### `[internal_api]`

`enabled` (false), `auth_method` (`shared_secret`), and `shared_secret` (redacted in `Debug`).
A served internal API requires a non-empty secret.

### `[providers.<name>]`

`adapter` (`oidc` | `apple`) plus adapter-specific fields captured via a flattened
`extra: HashMap<String, toml::Value>`. Every endpoint field (`issuer`, `jwks_uri`,
`token_endpoint`, `revocation_endpoint`) must be `https`. See
[05-provider-system.md](05-provider-system.md).

## Defaults summary

| Setting | Default |
|---|---|
| `server.host` / `port` / `role` / `request_timeout` | `0.0.0.0` / `8080` / `all` / `"30s"` |
| `server.issuer`, `token.audience` | `https://auth.example.com` / `https://api.example.com` *(replace for deployment)* |
| `registration.mode` | `open` |
| `token.access_token_ttl` / `refresh_token_ttl` | `15m` / `30d` |
| `audit.adapter` / `blocking_threshold` / `emit_threshold` | `noop` / `warning` / `info` |
| `telemetry.enabled` / `exporter` / `sample_rate` | `false` / `none` / `1.0` |
| `user_sync.enabled`, `internal_api.enabled` | `false`, `false` |

## Assumptions and open questions

### Assumptions

- Secrets and per-deployment values are supplied through the environment and referenced via
  `${VAR}`; secrets are never committed to a TOML file.
- Config is read once at startup; changing it requires a restart.

### Decisions

- *Closed configuration domains.* **Security-relevant configuration is narrowed during one
  resolver pass, not interpreted as arbitrary strings at request time.** Invalid values fail
  startup and every runtime shape observes the same decision.
- *Required non-empty issuer and audience.* **`server.issuer` and `token.audience` are closed
  non-empty domains.** The committed HTTPS values are deployment placeholders, not identities to
  use in production.
- *Secrets redacted in Debug.* **`WebhookConfig.secret` and `InternalApiConfig.shared_secret`
  have custom `Debug`.** Prevents secret leakage through log lines that dump config.
- *Separate session repository section.* **`[session_repository]` is optional and overrides
  only session storage.** Enables split topologies (SQL users + Valkey/LMDB sessions) without
  duplicating the user-store config.

### Open questions

- None.
