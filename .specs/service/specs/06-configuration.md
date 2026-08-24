# Configuration

**Status:** Implemented · **Date:** 2026-08-05 · **Owner:** Ant Stanley · **Scope:** crates/core/src/config.rs, config/, crates/server/src/bootstrap.rs

One TOML file drives the whole service. `AppConfig` (and its nested structs) in
`crates/core/src/config.rs` deserializes it; every section uses `#[serde(default)]`, so any
omitted section falls back to its defaults.

## Loading order

Configuration reaches a running service through exactly one pipeline. An entry point chooses
which *sources* it layers; everything after the merge is the shared resolve, which every entry
point calls and none can bypass.

Sources, lowest precedence first:

1. `config/default.toml` — compiled-in defaults (committed; see below).
2. `config/{OIDC_EXCHANGE_ENV}.toml` — deep-merged overlay when `OIDC_EXCHANGE_ENV` is set
   (e.g. `production`, `sqlite-only`); tables merge recursively, scalars and arrays replace.
   File-backed entry points only.
3. `OIDC_EXCHANGE__{section}__{key}` environment variables — structural overrides reaching
   every config path, including map-valued sections. A double underscore separates path
   segments and each segment is lowercased; a single underscore stays inside its segment
   (`…__MY_IDP__…` targets `providers.my_idp`), so keys whose names themselves contain `__`
   cannot be addressed from the environment.

The shared resolve then runs over the merged tree, for every entry point:

4. `${VAR_NAME}` placeholders anywhere in the merged config are resolved from the environment
   (see [Placeholder resolution](#placeholder-resolution)).
5. The resolved config is validated — `server.role`, the duration strings, allowlist entry
   shape, and a non-empty `internal_api.shared_secret` when the internal API will be served
   (see [Validation at load](#validation-at-load)) — and a failure aborts before any adapter or
   router is built.

Steps 4 and 5 are one function. Deserializing the merged tree yields the raw config; only the
resolve produces the `Config` the runtime consumes, so a code path that skipped resolution has
nothing to hand `build_service`.

## Placeholder resolution

`${VAR_NAME}` in any string value, at any depth, is replaced with that environment variable's
value. Resolution is total and fail-closed: every placeholder either resolves to a real value
or aborts the load with a `ConfigError`. Literal placeholder text never reaches a running
service.

| Input | Outcome |
| --- | --- |
| `${NAME}`, `NAME` set to a non-empty value | replaced with the value |
| `${NAME}`, `NAME` unset | `ConfigError` naming `NAME` and the config path; the load produces no config |
| `${NAME}`, `NAME` set to the empty string | `ConfigError` naming `NAME`, worded to distinguish "set but empty" from "unset" |
| `${` with no closing `}` within 256 bytes | `ConfigError` naming the config path and the malformed placeholder |
| `${}` — empty name | `ConfigError` naming the config path |
| `$${` | the escape: rewritten to a literal `${`, never looked up in the environment |

An empty variable is rejected rather than substituted because the fields this idiom exists for
are the ones where an empty value means "no protection": an unpopulated secret-manager
reference is a plumbing failure, not an operator's intent. A value that is genuinely meant to
be empty is expressed by omitting the key (defaults apply) or by writing `""` in the TOML.

After resolution, no config value may still contain an unescaped `${`. This holds as a
post-condition on the resolved tree, so a value carrying placeholder text is a load failure
whatever assembled it.

Errors raised during resolution or validation name the environment variable and the config
path, never the resolved value. `internal_api.shared_secret` and `user_sync.webhook.secret`
stay redacted on every error and diagnostic path, exactly as they are in `Debug`.

## Configuration entry points

| Entry point | Sources layered | Code |
| --- | --- | --- |
| Standalone server (hyper) | 1 + 2 + 3 | `crates/server/src/main.rs` → `bootstrap::load_config` |
| Lambda runtime (same binary, `AWS_LAMBDA_RUNTIME_API` present) | 1 + 2 + 3 | `crates/server/src/main.rs` → `bootstrap::load_config` |
| `config check` subcommand | 1 + 2 + 3, or a single named file | `crates/server/src/main.rs` |
| FFI inline TOML (`OidcExchange::new`) | the supplied document + 3 | `crates/ffi/src/lib.rs` → `bootstrap::parse_config` |
| FFI file (`OidcExchange::from_file`) | the named file + 3 | reads the file, then `new` |
| Node binding (napi) | via the FFI entry points | `bindings/nodejs/src/lib.rs` |
| Python binding (PyO3) | via the FFI entry points | `bindings/python/src/lib.rs` |
| `@oidc-exchange/lambda` handler | via the Node binding | `bindings/lambda/src/index.ts` |

Every row ends in the same resolve, so placeholder handling, override handling, and rejection
behaviour are identical across channels. The `OIDC_EXCHANGE_ENV` overlay is the one legitimate
difference: it applies only where the service selects its own files, and an FFI caller supplies
the whole document, so there is nothing to overlay it onto.

## Pre-flight check (`oidc-exchange config check`)

```
oidc-exchange config check [--dir <config-dir>] [--file <path>]
```

`config check` layers the sources for the shape being checked — `--dir` (default `config/`) for
the server layering, `--file` for the single-document layering the bindings use — runs the same
resolve, and exits without constructing an adapter, binding a socket, or writing anything.
Exit `0` prints a summary of the resolved configuration with every secret-bearing field
rendered through its redacting `Debug`; any `ConfigError` exits non-zero with the message the
server would have printed at startup. It is the supported way to prove that a deployment's
environment satisfies its placeholders before the deployment happens.

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

## Validation at load

After merging and placeholder resolution, the shared resolve validates the result and refuses
to produce a config on failure (`ConfigError`):

- `server.role` must be `all`, `exchange`, or `admin`.
- Duration strings for request and token lifetimes must parse and fit their accepted bounds.
- Domain-allowlist entries must be exact domains or `*.domain` wildcards.
- A served internal API requires a non-empty `internal_api.shared_secret`.

Validation is a step of the shared resolve, so it runs identically on every entry point in
[Configuration entry points](#configuration-entry-points) — including config supplied as a
string through the FFI bindings (`bootstrap::parse_config`).

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
