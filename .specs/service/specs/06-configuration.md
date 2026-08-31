# Configuration

**Status:** Implemented · **Date:** 2026-08-31 · **Owner:** Ant Stanley · **Scope:** crates/core/src/config.rs, crates/server/src/bootstrap.rs, config/

One TOML file drives the whole service. `RawConfig` mirrors the merged TOML input; `AppConfig`
is the resolved configuration held by the service. Every entry point that constructs a running
service uses the same resolver, so a configuration that cannot be validated never reaches a
request path.

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
5. The merged tree is deserialized as `RawConfig`, merged onto the committed defaults, and
   narrowed by `AppConfig::resolve` into the typed configuration the service runs on; any value
   outside its closed domain is rejected (see [Validation at load](#validation-at-load)) and a
   failure aborts before any adapter or router is built.

Steps 4 and 5 are one function. Deserializing the merged tree yields only the raw config; the
resolve alone produces the `AppConfig` the runtime consumes, so a code path that skipped
resolution has nothing to hand `build_service`.

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
| `config check` subcommand | 1 + 2 + 3, a single named file, or a bare file with no environment sources | `crates/server/src/main.rs` |
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
oidc-exchange config check [<path>] [--dir <config-dir>] [--file <path>]
```

`config check` layers the sources for the shape being checked — `--dir` (default `config/`) for
the server layering, `--file` for the single-document layering the bindings use — runs the same
resolve, and exits without constructing an adapter, binding a socket, or writing anything.
Exit `0` prints a summary of the resolved configuration with every secret-bearing field
rendered through its redacting `Debug`; any `ConfigError` exits non-zero with the message the
server would have printed at startup. It is the supported way to prove that a deployment's
environment satisfies its placeholders before the deployment happens.

A bare `config check <path>` instead checks the named file against the committed defaults
through the side-effect-free resolver, consulting no environment source at all — no overlays,
no `OIDC_EXCHANGE__` overrides, no `${VAR}` lookups. Use it to validate a fully materialized
deployment file in isolation.

## Committed default (`config/default.toml`)

```toml
[server]
host = "0.0.0.0"
port = 8080
issuer = "https://auth.example.com"
role = "all"
request_timeout = "30s"
trusted_proxies = []
trusted_proxy_hops = 1

[registration]
mode = "open"

[token]
access_token_ttl = "15m"
refresh_token_ttl = "30d"
audience = "https://api.example.com"
refresh_rotation = true
refresh_rotation_grace = "10s"
refresh_reuse_retention = "24h"

[session_repository]
cleanup_interval = "1h"

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

The default is deliberately minimal — no key manager, no repository, no providers — but it
is no longer silent: audit events go to stdout by default. Its HTTPS issuer and audience are
deliberately documentation placeholders, not production identity values. A deployment must
replace them with its own non-empty values before it can issue tokens for its namespace. A
service that would sign tokens carrying `iss: ""` and `aud: ""` is not usefully runnable, and
empty values are not representable in `AppConfig`.

## Sections

### `[server]`

`host` (`0.0.0.0`), `port` (`8080`), `issuer` (**required** — the `iss` claim and discovery
issuer; an absolute `https` URL, no default), `role` (`all` | `exchange` | `admin`, default
`all`), `request_timeout` (humantime duration string like the token TTLs, default `"30s"`) —
the per-request timeout the server's timeout layer enforces — and `base_path` (optional, default
unset — a leading prefix such as `/prod` stripped from incoming request paths before routing;
honored in both Lambda and server mode, though it exists chiefly for API Gateway stages and
mount prefixes). `trusted_proxies` (CIDR list, default empty) and `trusted_proxy_hops`
(default `1`, at most `16`) govern client-address resolution: the hop is counted from the
right of `X-Forwarded-For` only when the observed peer is in `trusted_proxies`; with the
default empty list, forwarding headers are not trusted.

### `[registration]`

`mode` (`open` | `existing_users_only`, default `open`) is a closed domain: an unrecognised
value is a config-load error, never a silent selection of `open`. Optional `domain_allowlist`
contains exact or `*.domain` wildcard entries; entries are ASCII only.

### `[token]`

`access_token_ttl` (`"15m"`), `refresh_token_ttl` (`"30d"`), `audience` (**required**,
non-empty — the `aud` claim of every issued access token), optional `custom_claims`
(`HashMap<String, String>` of claim templates; see [03-service-flows.md](03-service-flows.md)),
`refresh_rotation` (bool, default `true`), `refresh_rotation_grace` (duration string,
default `"10s"`) and `refresh_reuse_retention` (duration string, default `"24h"`).

`refresh_rotation_grace` is the window in which the immediately-preceding generation is
still redeemable; config resolution rejects a value above `60s`, because the window is a
deliberate weakening and an unbounded one is indistinguishable from no rotation.
`refresh_reuse_retention` is how long a retired generation is remembered so its
re-presentation raises an alarm; it is capped per record at the family's own `expires_at`.
Both durations are narrowed at load, so an unparseable or zero value fails config
resolution.

### `[grants]`
Which grants `/token` serves and the parameters of the direct ID-token grant's replay
protection. `id_token` (bool, default `false`) — whether the direct ID-token grant is
served at all. `nonce_ttl` (humantime duration, default `"10m"`) — how long a nonce minted
for the direct grant remains claimable. `max_assertion_lifetime` (humantime duration,
default `"1h"`) — the ceiling on an accepted provider ID token's remaining lifetime; an
assertion with longer to live is refused. The authorization-code and refresh-token grants
are always served and have no switch. Both durations are validated at startup by
`AppConfig::validate`, so an unparseable value fails config load.

### `[audit]`
`adapter` (`noop` | `stdout` | `stderr` | `auto` | `sqs`, default `stdout`),
`blocking_threshold` (syslog severity name, default `warning`), `emit_threshold` (syslog
severity name, default `info`) — events with a severity strictly less severe than the
threshold are not emitted at all, independently of the blocking decision — and `durability`
(`observe` | `enforce`, default `observe`) for mandatory security-event write failures;
optional `[audit.sqs] { queue_url, region }`.

### `[rate_limit]`
`enabled` (default `true`), `store` (`in_process` | `none`, default `in_process`), `window`
(default `"1m"`), per-window budgets `per_ip` (60), `per_ip_failures` (10), `per_subject`
(10), `per_provider` (600), `max_concurrent_requests` (256), and `max_entries` (10000).
Budgets are per key and per process; zero disables a scope. `per_ip_failures` is consumed only
for authentication failures, preserving the normal IP allowance for legitimate requests.

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
`[session_repository.valkey] { url, key_prefix? }` or `[session_repository.lmdb] { path,
max_size_mb? }`. Absent → sessions live in the `[repository]` store. `cleanup_interval`
(duration string, default `"1h"`) — how often the long-lived runtimes run
`cleanup_expired_sessions` ([04-http-api.md](04-http-api.md) → Bootstrap). The sweep covers
`sessions` and `retired_refresh_tokens` alike; on the natively-expiring stores (DynamoDB
TTL, Valkey key expiry) it is a cheap backstop for whatever native expiry has not yet
reaped.

### `[user_sync]`

`enabled` (bool), `adapter` (`webhook`), and `[user_sync.webhook] { url, secret, timeout?,
retries? }`. `url` must be `https` because the payload carries the full user record. The
`secret` is a `Secret<String>` and cannot be formatted.

### `[telemetry]`

`enabled` (false), `exporter` (`none` | `stdout` | `otlp` | `xray` | `prometheus`), optional `endpoint`,
`service_name`, `sample_rate` (default 1.0), and `protocol`.

### `[internal_api]`
`enabled` (false — internal routes are not mounted unless true, regardless of `server.role`;
a `role = "admin"` instance with the flag off serves only `/health`), `auth_method`
(`shared_secret`), `shared_secret` (a `Secret<String>`; it cannot be formatted, must be
non-empty when the internal API is served, and internal auth compares it in constant time via
`subtle`).

### `[providers.<name>]`

`adapter` (`oidc` | `apple`) plus adapter-specific fields captured via a flattened
`extra: HashMap<String, toml::Value>`. Every endpoint field (`issuer`, `jwks_uri`,
`token_endpoint`, `revocation_endpoint`) must be `https`. `endpoint_origins` is an optional
array of `scheme "://" host [":" port]` origins that a discovery document is permitted to
name in addition to the issuer's own origin and the origins of any explicitly configured
endpoint; each entry must parse as an `https` origin with no path, query, or fragment (the
typed lift and strict per-entry validation run in the server's `provider_config_to_oidc`).
It defaults to empty, which pins a provider to its issuer's origin. While the
endpoint-origin check runs in its shipped warning mode, an undeclared origin logs a
structured warning naming the endpoint and the permitted set and the deployment is served
unchanged; rejecting undeclared origins is a separate future release-owner decision after
one release of that telemetry.

Two optional oidc-adapter keys govern how the adapter derives
`IdentityClaims.email_verified` for providers that do not emit the standard
`email_verified` claim (Microsoft Entra ID v2.0 is the motivating case):
`email_verified_claim` (non-empty string, at most 64 characters — read the named claim,
bool-or-string coerced, when the standard claim is absent) and `trust_email_verified`
(TOML boolean, default `false` — treat a non-empty `email` claim as verified when the
standard claim is absent). An explicit `email_verified` claim from the provider always
takes precedence, setting both keys on one provider block is a config error, and both
are validated in the same `provider_config_to_oidc` lift as the other adapter-specific
fields — a set-but-mistyped value fails registry build rather than being coerced or
ignored. See
[05-provider-system.md](05-provider-system.md#email-verification-overrides).

## Validation at load

Configuration is parsed in two stages. `RawConfig` mirrors the merged TOML input and is only an
intermediate; `AppConfig` is what the service holds, and its security-relevant fields are
enums, newtypes, and constrained URLs rather than arbitrary strings. After merging and
placeholder resolution, `AppConfig::resolve` constructs the narrowed types, returning a
`ConfigError` naming the offending field. A value that cannot be narrowed aborts startup; there
is no permissive fallback and no per-request re-parse.

The closed domains:

| Field | Type | Domain |
|---|---|---|
| `server.role` | `ServerRole` | `all` \| `exchange` \| `admin` |
| `server.issuer` | `HttpsUrl` | required, non-empty, absolute `https` URL |
| `registration.mode` | `RegistrationMode` | `open` \| `existing_users_only` |
| `registration.domain_allowlist[]` | `AsciiDomainPattern` | exact domain or `*.`-prefixed wildcard; non-ASCII rejected |
| `token.audience` | `NonEmptyString` | required, non-empty |
| `token.access_token_ttl`, `token.refresh_token_ttl`, `server.request_timeout` | `Duration` | `<integer><s\|m\|h\|d>`, no overflow |
| `token.refresh_rotation_grace` | `Duration` | strictly positive, at most `60s` |
| `token.refresh_reuse_retention`, `session_repository.cleanup_interval` | `Duration` | strictly positive |
| `key_manager.local.algorithm` | `SigningAlgorithm` | `EdDSA` — the only algorithm the local adapter can produce |
| `key_manager.kms.algorithm` | `SigningAlgorithm` | `RS256` \| `RS384` \| `RS512` \| `PS256` \| `PS384` \| `PS512` \| `ES256` \| `ES384` \| `ES512` (JWS names, RFC 7518 §3.1 — not AWS `SigningAlgorithmSpec` names) |
| `audit.adapter` | `AuditAdapter` | `noop` \| `stdout` \| `stderr` \| `auto` \| `sqs` |
| `audit.blocking_threshold`, `audit.emit_threshold` | `AuditSeverity` | syslog severity name |
| `telemetry.exporter` | `TelemetryExporter` | `none` \| `stdout` \| `otlp` \| `xray` \| `prometheus` |
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

Validation is a step of the shared resolve, so it runs identically on every entry point in
[Configuration entry points](#configuration-entry-points) — including config supplied as a
string through the FFI bindings (`bootstrap::parse_config`).

## Defaults summary

| Setting | Default |
|---|---|
| `server.host` / `port` / `role` / `request_timeout` | `0.0.0.0` / `8080` / `all` / `"30s"` |
| `server.issuer`, `token.audience` | `https://auth.example.com` / `https://api.example.com` *(replace for deployment)* |
| `server.trusted_proxies` / `trusted_proxy_hops` | `[]` / `1` |
| `registration.mode` | `open` |
| `token.access_token_ttl` / `refresh_token_ttl` | `15m` / `30d` |
| `grants.id_token` | `false` |
| `grants.nonce_ttl` / `max_assertion_lifetime` | `10m` / `1h` |
| `token.refresh_rotation` / `refresh_rotation_grace` / `refresh_reuse_retention` | `true` / `"10s"` / `"24h"` |
| `session_repository.cleanup_interval` | `"1h"` |
| `audit.adapter` / `blocking_threshold` / `emit_threshold` / `durability` | `stdout` / `warning` / `info` / `observe` |
| `rate_limit.enabled` / `store` / `window` / `max_concurrent_requests` | `true` / `in_process` / `"1m"` / `256` |
| `rate_limit.per_ip` / `per_ip_failures` / `per_subject` / `per_provider` | `60` / `10` / `10` / `600` |
| `telemetry.enabled` / `exporter` / `sample_rate` | `false` / `none` / `1.0` |
| `user_sync.enabled`, `internal_api.enabled` | `false`, `false` |

## Assumptions and open questions

### Assumptions

- Secrets and per-deployment values are supplied through the environment and referenced via
  `${VAR}`; secrets are never committed to a TOML file.
- Config is read once at startup; changing it requires a restart.
- `audit.adapter` must be a known non-empty adapter, `audit.durability` must be `observe` or
  `enforce`, trusted proxies must be CIDRs, `trusted_proxy_hops` must be 1–16, and the
  rate-limit window, store, budgets, entry bound, and concurrency bound are validated at load
  time. `rate_limit.enabled = true` requires `store = "in_process"`.
- Where a rate limit must hold globally (Lambda or horizontally scaled servers), an edge
  gateway/WAF provides it; the in-process limiter is a per-process backstop.

### Decisions

- *Closed configuration domains.* **Security-relevant configuration is narrowed during one
  resolver pass, not interpreted as arbitrary strings at request time.** Invalid values fail
  startup and every runtime shape observes the same decision.
- *Required non-empty issuer and audience.* **`server.issuer` and `token.audience` are closed
  non-empty domains.** The committed HTTPS values are deployment placeholders, not identities to
  use in production.
- *`serde(default)` everywhere.* **Every config section has defaults.** A minimal TOML boots,
  and adding a field never breaks deserialization of existing files.
- *Secrets are unprintable by type.* **Credential-bearing config values are `Secret<T>`, a
  newtype implementing neither `Debug` nor `Display`.** `WebhookConfig.secret`,
  `InternalApiConfig.shared_secret`, and `OidcProviderConfig.client_secret` previously relied on
  hand-written `Debug` impls rendering `"<redacted>"`; the newtype makes a leak a compile error
  rather than a per-type discipline — the enclosing structs' `Debug` impls still elide the
  secret field, but forgetting the elision now fails to compile instead of leaking.
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


## Runtime parity update

`host` (`0.0.0.0`), `port` (`8080`), `issuer` (the `iss` claim / discovery issuer, default
empty), `role` (`all` | `exchange` | `admin`, default `all`), `request_timeout` (humantime
duration string like the token TTLs, default `"30s"`) — the per-request timeout the server's
timeout layer enforces; `base_path` (optional, default unset — a leading prefix such as
`/prod` stripped from incoming request paths at a segment boundary before routing, honored in
server, Lambda, and every embedded runtime); `max_request_body_bytes` (default `2097152`) —
the request body ceiling the server's body-limit layer enforces and every binding enforces
before it buffers.

`base_path` is normalised and validated at config load: an empty string and `"/"` both
resolve to unset, a value not starting with `/` is a startup error, and a trailing `/` is
trimmed. Validating once at startup is what lets the per-request path be free of assertions.
| `server.host` / `port` / `role` / `request_timeout` / `max_request_body_bytes` | `0.0.0.0` / `8080` / `all` / `"30s"` / `2097152` |
