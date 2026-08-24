# HTTP API, Roles, and Bootstrap

**Status:** Implemented · **Date:** 2026-08-22 · **Owner:** Ant Stanley · **Scope:** crates/server

The axum layer: routes, middleware, the `role`-based route/adapter selection, the
startup sequence, and the domain-error-to-HTTP mapping. Lives in `crates/server/src/`.

## Routes

### Public (mounted for roles `exchange` and `all`)

| Method | Path | Handler | Purpose |
|---|---|---|---|
| GET | `/health` | `health` | `{"status":"ok"}` — mounted for every role |
| POST | `/token` | `token` | exchange (`authorization_code`/`id_token`) and refresh (`refresh_token`) |
| POST | `/revoke` | `revoke` | RFC 7009 revocation of the session the presented credential names: 200 for invalid/unknown tokens, 503 on backend failure |
| GET | `/keys` | `keys` | JWKS: `{"keys":[<jwk>]}` from `KeyManager::public_jwk` |
| GET | `/.well-known/openid-configuration` | `openid_config` | discovery document |
| POST | `/nonce` | `nonce` | mint a single-use nonce for the direct ID-token grant; mounted only when `grants.id_token = true` |

### Internal (mounted for roles `admin` and `all` only when `internal_api.enabled = true`, behind Bearer auth)

These routes are mounted only when `internal_api.enabled = true` and `server.role` is
`admin` or `all`; with the flag false (the default) no internal routes exist regardless of
role. When mounted they sit behind Bearer auth (see Middleware stack).

| Method | Path | Purpose |
|---|---|---|
| GET | `/internal/stats` | aggregate user/session counts (`AdminStats`) |
| GET | `/internal/users` | list users, query `offset`/`limit` |
| POST | `/internal/users` | create user (`NewUser`) → 201 |
| GET | `/internal/users/{id}` | get user (404 if absent) |
| PATCH | `/internal/users/{id}` | update user (`UserPatch`; 404 if absent) |
| DELETE | `/internal/users/{id}` | soft-delete user (404 if absent) |
| GET | `/internal/users/{id}/claims` | read claims (404 if absent) |
| PUT | `/internal/users/{id}/claims` | replace claims (404 if absent) |
| PATCH | `/internal/users/{id}/claims` | merge claims (404 if absent) |
| DELETE | `/internal/users/{id}/claims` | clear claims (404 if absent) |

### POST /nonce

Takes no body and returns `{"nonce": "<base64url>", "expires_in": <seconds>}`. The nonce
is 32 random bytes, base64url-no-pad; only its SHA-256 hex digest is stored. The route is
unauthenticated by necessity — the caller holds no credential yet — and is not mounted at
all when the direct grant is disabled, so an operator who leaves the default in place
gains no new public surface.

### POST /token request

`application/x-www-form-urlencoded`. `grant_type` selects the flow:

```
# code exchange:  grant_type=authorization_code & code=… & redirect_uri=… & provider=google
# direct token:   grant_type=id_token & id_token=… & provider=google
#                 [& provider_access_token=…]
# refresh:        grant_type=refresh_token & refresh_token=…
```

The client names the provider (`provider=google`), not a raw issuer URL. Unknown
`grant_type` → `unsupported_grant_type`. Response body is `TokenResponse`
([01-domain-model.md](01-domain-model.md)).

The direct grant requires the ID token to carry a `nonce` claim whose value came from this
service's `POST /nonce`; the client passes that value into the provider's authentication
request and does not resend it here — the service reads it from the verified assertion.
`provider_access_token` is optional and carries the provider access token co-issued with
the ID token, so the `at_hash` binding can be verified. When `grants.id_token = false` an
`id_token` field is rejected with `unsupported_grant_type` whatever `grant_type` declares,
so the switch cannot be evaded by the field-presence branch selection.

### GET /.well-known/openid-configuration

Reports `issuer`, `jwks_uri` (`/keys`), `token_endpoint`, `revocation_endpoint`, supported
grant types (`authorization_code`, `refresh_token`, and — only when `grants.id_token =
true` — `id_token`; the document describes the grants the process actually serves),
`response_types_supported` (`code`), `subject_types_supported` (`public`), and
`id_token_signing_alg_values_supported` populated from `KeyManager::algorithm()`.

## Discovery / state

`AppState { service: Arc<AppService>, config: Arc<AppConfig> }` is axum's shared state,
extracted into every handler.

## Middleware stack

Applied to the router (`routes/mod.rs`), outermost first:

1. **Request ID** (`middleware/request_id.rs`) — reuse `X-Request-Id` or generate a UUIDv4;
   open a per-request `info_span` carrying `request_id` so all downstream logs — including
   the `server_error` detail log — inherit it; echo in the response header.
2. **Request timeout** (`tower_http::timeout::TimeoutLayer`) — abort any request that runs
   longer than `server.request_timeout` (default `30s`) and respond `408`. Sits inside the
   request-id layer, so a timeout response still carries the request id, and outside the
   rest of the stack, so the bound covers the remaining middleware and the handler.
3. **Audit context** (`middleware/audit_context.rs`) — extract `X-Forwarded-For`,
   `User-Agent`, `X-Device-Id` into an `AuditContext` request extension, which the `/token`
   and `/revoke` handlers pass into the core request structs so the stored session records
   `ip_address`/`user_agent`/`device_id` and audit events record `ip_address`/`user_agent`.
4. **Catch-panic** (`middleware/error_handler.rs`, tower `CatchPanicLayer`) — a panic becomes
   `500 {"error":"server_error","error_description":"internal server error"}`.

Internal routes mount only when `internal_api.enabled = true` and the role is `admin` or
`all`; with the flag false no internal routes are mounted regardless of role, so an
`admin`-role instance serves only `/health`. When mounted, they additionally pass through
**internal auth** (`middleware/internal_auth.rs`):
`Authorization: Bearer <secret>` compared to `internal_api.shared_secret` in constant time
(`subtle`); missing/wrong → `401`. A missing or empty secret is rejected at startup, never
discovered at request time.

## Service roles

`server.role` ∈ `{ all, exchange, admin }` (default `all`) controls both routing and which
adapters the bootstrap builds:

| Role | Routes | Adapters built |
|---|---|---|
| `exchange` | public + `/health` | user repo, session repo, key manager, providers, audit; user-sync → noop |
| `admin` | `/internal/*` (only when `internal_api.enabled = true`) + `/health` | user repo, session repo, audit, user-sync; key manager → noop, no providers |
| `all` | all of the above (`/internal/*` only when `internal_api.enabled = true`) | all |

This lets the latency-sensitive public exchange path and the low-traffic admin path scale and
be network-isolated independently from one binary.

## Bootstrap (`main.rs` + `bootstrap.rs`)

1. Handle the CLI surface and exit: `--version` prints the crate version; `config check`
   layers configuration sources and runs the same resolve as step 2, prints a redacted summary,
   and exits non-zero on any `ConfigError` without building adapters or binding a socket
   ([06-configuration.md](06-configuration.md)).
2. `bootstrap::load_config` — layer `config/default.toml`, the
   `config/{OIDC_EXCHANGE_ENV}.toml` overlay if set, and `OIDC_EXCHANGE__{section}__{key}` env
   overrides, then run the shared resolve: fail-closed `${VAR}` placeholder resolution followed
   by validation (role, TTLs, allowlist, internal-API secret —
   [06-configuration.md](06-configuration.md)).
3. `telemetry::init_telemetry` — install the tracing subscriber first so all later spans are
   captured ([07-telemetry-and-audit.md](07-telemetry-and-audit.md)).
4. `bootstrap::build_service` — construct adapters by role and assemble `AppService`.
5. `bootstrap::build_router` — build the axum router for the role with middleware and state.
6. Detect runtime: `AWS_LAMBDA_RUNTIME_API` present → the router is served through
   `lambda_http::run` as a tower service, accepting API Gateway REST/HTTP-API, Function URL,
   and ALB events; otherwise bind `server.host:server.port` and serve over hyper with graceful
   shutdown — SIGTERM or ctrl-c stops accepting connections and drains in-flight requests for
   up to a 10 s hard deadline, after which stragglers are aborted and the process exits. The
   middleware stack's request-timeout layer bounds slow clients at `server.request_timeout`
   (default 30 s). Both paths run the identical router, middleware stack, and `AppState`, and
   both strip a configured `server.base_path` prefix from incoming request paths before routing
   ([06-configuration.md](06-configuration.md)) — covering API Gateway stages and mount
   prefixes.

`crates/ffi` layers its own sources into the same resolve and then calls the same
`build_service` / `build_router` path, so in-process bindings get identical configuration
semantics, routing, and middleware.

## Error mapping (`error.rs`)

`ApiError` wraps the domain `Error` (plus `UnsupportedGrantType`) and renders
`{"error": <code>, "error_description": <detail>}` — an OAuth-style error envelope; codes
beyond RFC 6749 §5.2 (`not_found`) use the same shape:

| Domain error | HTTP | `error` |
|---|---|---|
| `InvalidGrant` | 400 | `invalid_grant` |
| `NotFound` | 404 | `not_found` |
| `InvalidToken` | 401 | `invalid_token` |
| `InvalidRequest`, `UnknownProvider` | 400 | `invalid_request` |
| `AccessDenied`, `UserSuspended` | 403 | `access_denied` |
| `Unauthorized` | 401 | `unauthorized` |
| `UnsupportedGrantType` | 400 | `unsupported_grant_type` |
| `Conflict` | 409 | `conflict` |
| `ProviderError` | 502 | `server_error` |
| `ProviderTimeout` | 504 | `server_error` |
| `StoreError`, `KeyError`, `AuditError`, `SyncError`, `ConfigError` | 500 | `server_error` |

`server_error` responses (500/502/504) log the internal detail via `tracing::error!` —
inside the request span, so the log carries the request id — and return a generic message;
infrastructure detail is never leaked to the client.

## Assumptions and open questions

### Assumptions

- A reverse proxy or gateway terminates TLS and may set `X-Forwarded-For`; the service reads
  but does not validate that header's trust chain.
- The internal API is reachable only from trusted callers (admin UI, scripts) on a private
  network; the shared secret is its only authentication.

### Decisions

- *Role-driven wiring.* **`server.role` selects routes and adapters together.** A single
  build artifact covers public-only, admin-only, and combined deployments.
- *Constant-time secret compare.* **Internal auth uses `subtle`.** Avoids timing oracles on
  the shared secret.
- *200 for token state, 503 for infrastructure.* **`/revoke` returns 200 whether the token
  was revoked, invalid, or unknown, and 503 when the backend fails.** RFC 7009 forbids
  leaking whether a token existed (§2.2) but permits 503 when the server cannot handle the
  request (§2.2.1); a client must never be told a live session is dead.
- *Revocation reaches one session.* **`/revoke` removes the session named by the credential
  presented and nothing else.** The endpoint is unauthenticated by design (RFC 7009 §2.1
  permits it, and the token is the credential), so its blast radius must be the credential's
  own; a public endpoint that can end every session of a named subject is a
  denial-of-service primitive for anyone who scavenges one token. Ending all of a user's
  sessions is an operator action and lives behind internal auth.
- *Discovery reflects the live key.* **`id_token_signing_alg_values_supported` comes from the
  configured `KeyManager`.** The advertised algorithm always matches the signing key.

### Open questions

- Telemetry exporters `otlp`/`xray` are accepted in config but currently fall back to JSON
  logging; the tower OTEL HTTP-span layer is not yet wired. See
  [07-telemetry-and-audit.md](07-telemetry-and-audit.md).
