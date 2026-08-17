# HTTP API, Roles, and Bootstrap

**Status:** Implemented · **Date:** 2026-08-05 · **Owner:** Ant Stanley · **Scope:** crates/server

The axum layer: routes, middleware, the `role`-based route/adapter selection, the
startup sequence, and the domain-error-to-HTTP mapping. Lives in `crates/server/src/`.

## Routes

### Public (mounted for roles `exchange` and `all`)

| Method | Path | Handler | Purpose |
|---|---|---|---|
| GET | `/health` | `health` | `{"status":"ok"}` — mounted for every role |
| POST | `/token` | `token` | exchange (`authorization_code`/`id_token`) and refresh (`refresh_token`) |
| POST | `/revoke` | `revoke` | RFC 7009 revocation: 200 for invalid/unknown tokens, 503 on backend failure |
| GET | `/keys` | `keys` | JWKS: `{"keys":[<jwk>]}` from `KeyManager::public_jwk` |
| GET | `/.well-known/openid-configuration` | `openid_config` | discovery document |

### Internal (mounted for roles `admin` and `all`, behind Bearer auth)

| Method | Path | Purpose |
|---|---|---|
| GET | `/internal/stats` | aggregate user/session counts (`AdminStats`) |
| GET | `/internal/users` | list users, query `offset`/`limit` |
| POST | `/internal/users` | create user (`NewUser`) → 201 |
| GET | `/internal/users/{id}` | get user (404 if absent) |
| PATCH | `/internal/users/{id}` | update user (`UserPatch`) |
| DELETE | `/internal/users/{id}` | soft-delete user |
| GET | `/internal/users/{id}/claims` | read claims |
| PUT | `/internal/users/{id}/claims` | replace claims |
| PATCH | `/internal/users/{id}/claims` | merge claims |
| DELETE | `/internal/users/{id}/claims` | clear claims |

### POST /token request

`application/x-www-form-urlencoded`. `grant_type` selects the flow:

```
# code exchange:  grant_type=authorization_code & code=… & redirect_uri=… & provider=google
# direct token:   grant_type=id_token & id_token=… & provider=google
# refresh:        grant_type=refresh_token & refresh_token=…
```

The client names the provider (`provider=google`), not a raw issuer URL. Unknown
`grant_type` → `unsupported_grant_type`. Response body is `TokenResponse`
([01-domain-model.md](01-domain-model.md)).

### GET /.well-known/openid-configuration

Reports `issuer`, `jwks_uri` (`/keys`), `token_endpoint`, `revocation_endpoint`, supported
grant types (`authorization_code`, `refresh_token`), `response_types_supported` (`code`),
`subject_types_supported` (`public`), and `id_token_signing_alg_values_supported` populated
from `KeyManager::algorithm()`.

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
   `User-Agent`, `X-Device-Id` into an `AuditContext` request extension.
4. **Catch-panic** (`middleware/error_handler.rs`, tower `CatchPanicLayer`) — a panic becomes
   `500 {"error":"server_error","error_description":"internal server error"}`.

Internal routes additionally pass through **internal auth** (`middleware/internal_auth.rs`):
`Authorization: Bearer <secret>` compared to `internal_api.shared_secret` in constant time
(`subtle`); missing/wrong/unconfigured → `401`.

## Service roles

`server.role` ∈ `{ all, exchange, admin }` (default `all`) controls both routing and which
adapters the bootstrap builds:

| Role | Routes | Adapters built |
|---|---|---|
| `exchange` | public + `/health` | user repo, session repo, key manager, providers, audit; user-sync → noop |
| `admin` | `/internal/*` + `/health` | user repo, session repo, audit, user-sync; key manager → noop, no providers |
| `all` | all of the above | all |

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
   by validation ([06-configuration.md](06-configuration.md)).
3. `telemetry::init_telemetry` — install the tracing subscriber first so all later spans are
   captured ([07-telemetry-and-audit.md](07-telemetry-and-audit.md)).
4. `bootstrap::build_service` — construct adapters by role and assemble `AppService`.
5. `bootstrap::build_router` — build the axum router for the role with middleware and state.
6. Detect runtime: `AWS_LAMBDA_RUNTIME_API` present → Lambda mode; otherwise bind
   `server.host:server.port` and serve over hyper with graceful shutdown — SIGTERM or
   ctrl-c stops accepting connections and drains in-flight requests for up to a 10 s hard
   deadline, after which stragglers are aborted and the process exits. The middleware
   stack's request-timeout layer bounds slow clients at `server.request_timeout`
   (default 30 s).

`crates/ffi` layers its own sources into the same resolve and then calls the same
`build_service` / `build_router` path, so in-process bindings get identical configuration
semantics, routing, and middleware.

## Error mapping (`error.rs`)

`ApiError` wraps the domain `Error` (plus `UnsupportedGrantType`) and renders
`{"error": <code>, "error_description": <detail>}` (RFC 6749 §5.2):

| Domain error | HTTP | `error` |
|---|---|---|
| `InvalidGrant` | 400 | `invalid_grant` |
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
- *Discovery reflects the live key.* **`id_token_signing_alg_values_supported` comes from the
  configured `KeyManager`.** The advertised algorithm always matches the signing key.

### Open questions

- Telemetry exporters `otlp`/`xray` are accepted in config but currently fall back to JSON
  logging; the tower OTEL HTTP-span layer is not yet wired. See
  [07-telemetry-and-audit.md](07-telemetry-and-audit.md).
