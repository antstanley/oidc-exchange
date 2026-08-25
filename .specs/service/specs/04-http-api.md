# HTTP API, Roles, and Bootstrap

**Status:** Implemented · **Date:** 2026-08-23 · **Owner:** Ant Stanley · **Scope:** crates/server

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
| POST | `/internal/sessions/cleanup` | run `cleanup_expired_sessions` once; returns `{ "deleted": <count> }` |
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

`application/x-www-form-urlencoded`. `grant_type` is required and **binding**: it alone
selects the flow, and a request may carry only the parameters its declared grant defines.

```
# code exchange:  grant_type=authorization_code & code=… & redirect_uri=… & provider=google
# direct token:   grant_type=id_token & id_token=… & provider=google
#                 [& provider_access_token=…]
# refresh:        grant_type=refresh_token & refresh_token=…
```

| `grant_type` | Required parameters | Rejected if present |
|---|---|---|
| `authorization_code` | `provider`, `code`, `redirect_uri` | `id_token`, `refresh_token` |
| `id_token` | `provider`, `id_token` | `code`, `redirect_uri`, `refresh_token` |
| `refresh_token` | `refresh_token` | `provider`, `code`, `redirect_uri`, `id_token` |

A parameter this server knows but that belongs to another grant is **rejected**, not ignored
— RFC 6749 §3.2's "MUST ignore unrecognized request parameters" covers parameters the server
does not recognise, which these are not. Parameters outside this set entirely are ignored.

The client names the provider (`provider=google`), not a raw issuer URL. The handler parses
the form into a `TokenGrant` before calling the service, so a request whose fields do not
match its declared grant never reaches `AppService`. Response body is `TokenResponse`
([01-domain-model.md](01-domain-model.md)); it carries a `refresh_token` on every grant,
including `refresh_token`, and a client must discard the token it presented once it holds
the replacement (RFC 6749 §6). With `token.refresh_rotation = false` the refresh grant
returns no `refresh_token` and the presented one stays valid.

The direct grant requires the ID token to carry a `nonce` claim whose value came from this
service's `POST /nonce`; the client passes that value into the provider's authentication
request and does not resend it here — the service reads it from the verified assertion.
`provider_access_token` is optional and carries the provider access token co-issued with
the ID token, so the `at_hash` binding can be verified. When `grants.id_token = false` an
`id_token` field is rejected with `unsupported_grant_type` whatever `grant_type` declares,
so the switch cannot be evaded by the field-presence branch selection.

Token-endpoint errors, in the RFC 6749 §5.2 envelope:

| Condition | HTTP | `error` | `error_description` |
|---|---|---|---|
| `grant_type` absent | 400 | `invalid_request` | `missing required parameter: grant_type` |
| `grant_type` present but not one of the three (including empty) | 400 | `unsupported_grant_type` | `The grant_type parameter is not supported` |
| an `id_token` field present while `grants.id_token = false` | 400 | `unsupported_grant_type` | `The grant_type parameter is not supported` |
| a required parameter of the declared grant absent | 400 | `invalid_request` | `missing required parameter: <name>` |
| a parameter of another grant present | 400 | `invalid_request` | `<name> is not a parameter of the <grant_type> grant` |

Every `/token` response — success and error alike — carries `Cache-Control: no-store` and
`Pragma: no-cache` (RFC 6749 §5.1 and §5.2; OpenID Connect Core §3.1.3.3). The body of a
successful response *is* the credential — the signed access token and, on exchange, the
plaintext refresh token, whose only copy in flight is that response — and the header is
the origin's sole mechanism for marking it non-storable: a `200` to a `POST` is
heuristically cacheable under RFC 9111 §3, so without the directive a conforming shared
cache is *permitted to store* the credential even though it may never reuse it. The
directives are applied by a route-scoped layer (`middleware/cache_control.rs`) on the
credential-bearing route group — `/token` and `/revoke` — not per handler, so the next
credential-returning route inherits them by being mounted in the group. `/revoke`'s
responses carry no token and RFC 7009 imposes no cache requirement; it is in the group
because its *requests* carry credentials and because a group-level property survives the
refactors that a per-handler memory does not. `/keys` and
`/.well-known/openid-configuration` sit outside the group and keep their own (cacheable)
policy.

### GET /.well-known/openid-configuration

Reports `issuer`, `jwks_uri` (`/keys`), `token_endpoint`, `revocation_endpoint`, supported
grant types (`authorization_code`, `refresh_token`, and — only when `grants.id_token =
true` — `id_token`; the document describes the grants the process actually serves),
`response_types_supported` (`code`), `subject_types_supported` (`public`), and
`id_token_signing_alg_values_supported` populated from `KeyManager::algorithm()`.

## Discovery / state

`AppState { service: Arc<AppService>, config: Arc<AppConfig>, rate_limiter: Arc<dyn RateLimiter> }`
is axum's shared state, extracted into every handler. The retained configured limiter is shared
by all public route middleware instances so per-IP fixed-window state survives requests; the
service retains its configured limiter separately for provider and subject enforcement.

## Middleware stack

Applied to the router, outermost first:

1. **Request ID** (`middleware/request_id.rs`) — reuses an inbound `X-Request-Id` only when
   it is a plausible correlation identifier: non-empty, at most 128 bytes, and drawn from
   `[A-Za-z0-9_-]`. Anything else is discarded and a fresh UUIDv4 generated instead; the
   request is never failed over a malformed correlation header, and the rejected value is
   never logged. Opens a request span carrying `request_id` and echoes the response header.
2. **Request timeout** — bounds the rest of the stack and handler at
   `server.request_timeout` (default `30s`) and returns `408`.
3. **Audit context / client address** (`middleware/audit_context.rs`) — resolves `ClientAddr`.
   Under hyper, `ConnectInfo` supplies a `Peer`; Lambda uses the platform request-context
   source IP; FFI has no peer and records `Unknown`. When the observed peer is in
   `server.trusted_proxies`, the `X-Forwarded-For` entry `server.trusted_proxy_hops` from the
   right is parsed as `Forwarded`. Otherwise the peer remains authoritative. User-Agent and
   device-id values are truncated to 256 characters.
4. **Catch panic** — renders a safe `500` response.

Public routes additionally use, in route-layer execution order, the per-IP throttle, access
log, and concurrency guard. The throttle runs before handler/provider work; only `Peer` and
`Forwarded` values become rate-limit keys. A denial returns `429 slow_down` and
`Retry-After`; every direct per-IP denial also emits the mandatory `ThrottleExceeded`
`SecurityEvent` using the resolved `ClientAddr` and bounded User-Agent. Audit-sink failure is
recorded through the mandatory-channel durability contract but cannot replace or otherwise alter
the safe throttle `429`. Asserted or unknown addresses are not throttled. The access log records
method, matched path, status, safe OAuth error code, and address kind, never token/form/header
values. It runs inside the request-id middleware's request span and therefore inherits that span
(and its request-id correlation) for its tracing event. The semaphore-based concurrency guard
rejects saturation with `503`.

One layer is route-scoped rather than router-wide: **cache control**
(`middleware/cache_control.rs`) is mounted on the merged credential group (`/token`,
`/revoke` inside `public_routes`) and stamps `Cache-Control: no-store` / `Pragma: no-cache`
onto every response that group produces — see the `POST /token request` section.

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
   and ALB events; otherwise bind `server.host:server.port` and serve over hyper through
   `into_make_service_with_connect_info::<SocketAddr>()`, so middleware can classify the
   observed connection peer, with graceful shutdown — SIGTERM or ctrl-c stops accepting
   connections and drains in-flight requests for up to a 10 s hard deadline, after which
   stragglers are aborted and the process exits. The
   middleware stack's request-timeout layer bounds slow clients at `server.request_timeout`
   (default 30 s). Both paths run the identical router, middleware stack, and `AppState`, and
   both strip a configured `server.base_path` prefix from incoming request paths before routing
   ([06-configuration.md](06-configuration.md)) — covering API Gateway stages and mount
   prefixes.
7. Under a long-lived runtime — hyper, and a `crates/ffi` embedder whose host process
   persists — spawn the **session reaper**: a periodic task that calls
   `SessionRepository::cleanup_expired_sessions` every
   `session_repository.cleanup_interval` (default `"1h"`), logs the deleted count on every
   run — a silently dead reaper must be distinguishable from one with nothing to delete —
   and is aborted with the graceful-shutdown drain. Under Lambda there is no long-lived
   process to host the task; the reaper is not spawned, and the same control is reachable
   as `POST /internal/sessions/cleanup` for an external scheduler (EventBridge) to drive
   on the deployment's own cadence.

`crates/ffi` layers its own sources into the same resolve and then calls the same
`build_service` / `build_router` path, so in-process bindings get identical configuration
semantics, routing, and middleware. An embedder instance detects its host the same way
`main.rs` does (`AWS_LAMBDA_RUNTIME_API`) and hosts the session reaper on its own runtime
only when that host persists; inside a Lambda function it spawns none.

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
| `TooManyRequests` | 429 | `slow_down` |
| `ProviderError` | 502 | `server_error` |
| `ProviderTimeout` | 504 | `server_error` |
| `StoreError`, `KeyError`, `AuditError`, `SyncError`, `ConfigError` | 500 | `server_error` |

The status and `error` code are as tabulated; the `error_description` is **always**
`Error::client_description()` — a stable `&'static str` per variant, drawn from a small fixed
set, that never embeds caller input, library error text, provider key state, or cache
internals. The internal `reason`/`detail` an adapter composed is never published.
(`UnsupportedGrantType`, a route-level error with no domain counterpart, keeps its fixed
static description — generic by construction, so the same rule holds.)

Every mapped domain error — not only the `server_error` class — logs its full internal
`Display` via `tracing::error!` (5xx) or `tracing::warn!` (4xx) inside the request span, so
the log carries the request id and the operator loses no diagnostic power. A production
assertion checks that no rendered description ever equals the full `Display`, and a debug
assertion checks that it equals `err.client_description()` for every arm, generalising the
guard that previously protected only `server_error`.

The consequence for a caller is that an unknown `kid`, a bad signature, an expired token,
and a wrong audience are indistinguishable at `/token`: each is
`400 {"error":"invalid_grant","error_description":"the provided grant could not be validated"}`.
RFC 6749 §5.2 makes `error_description` optional and developer-facing, so genericising it
breaks no conformance.
A `429` carries `Retry-After` in seconds for the remainder of the current fixed window.
`slow_down` is RFC 8628 §3.5's token-endpoint rate-limit code.


## Assumptions and open questions

### Assumptions

- A reverse proxy or gateway terminates TLS. `X-Forwarded-For` is honoured only when the
  observed connection peer is inside `server.trusted_proxies`; with the shipped empty list
  it is not a rate-limit or authorization input.
- In-process rate-limit state is per process. A horizontally scaled deployment's effective
  budget grows with instance count; under Lambda it bounds each execution environment, so an
  API Gateway usage plan or WAF remains the global control.
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


## Runtime parity update

Applied to the router (`routes/mod.rs`), outermost first:

1. **Outer catch-panic** (`middleware/error_handler.rs`, tower `CatchPanicLayer`) — wraps
   the base-path service and everything inside it, so a panic in any layer becomes
   `500 {"error":"server_error","error_description":"internal server error"}` instead of a
   dropped connection or an unwind into an embedding host. It is the outermost guard, not a
   move of the inner one: moving the single guard outward would cost a caught handler panic
   its `x-request-id`, so the stack carries two.
2. **Base-path strip** (`middleware/base_path.rs`) — strip `server.base_path` at a
   path-segment boundary before the routing decision. `base_path` is normalised at config
   load, so the middleware never sees `""` or `"/"`, and no assertion runs on a request path.
3. **Request ID** (`middleware/request_id.rs`) — reuse `X-Request-Id` or generate a UUIDv4;
   open a per-request `info_span` carrying `request_id` so all downstream logs — including
   the `server_error` detail log — inherit it; echo in the response header.
4. **Request timeout** (`tower_http::timeout::TimeoutLayer`) — abort any request that runs
   longer than `server.request_timeout` (default `30s`) and respond `408`. Sits inside the
   request-id layer, so a timeout response still carries the request id, and outside the
   rest of the stack, so the bound covers the remaining middleware and the handler.
5. **Body limit** (`axum::extract::DefaultBodyLimit`) — reject a request body above
   `server.max_request_body_bytes` with `413`. Embedded hosts enforce the same number before
   they buffer, so one configured value bounds all five runtime shapes.
6. **Audit context** (`middleware/audit_context.rs`) — extract `X-Forwarded-For`,
   `User-Agent`, `X-Device-Id` into an `AuditContext` request extension, which the `/token`
   and `/revoke` handlers pass into the core request structs so the stored session records
   `ip_address`/`user_agent`/`device_id` and audit events record `ip_address`/`user_agent`.
7. **Inner catch-panic** (`middleware/error_handler.rs`, tower `CatchPanicLayer`) — nearest
   the handler, so a caught handler panic still passes back out through the request-id layer
   and its response carries `x-request-id`.

Internal routes mount only when `internal_api.enabled = true` and the role is `admin` or
`all`; with the flag false no internal routes are mounted regardless of role. When mounted,
they additionally pass through **internal auth** (`middleware/internal_auth.rs`):
`Authorization: Bearer <secret>` compared to `internal_api.shared_secret` in constant time
(`subtle`); missing/wrong → `401`. A missing or empty secret is rejected at startup, never
discovered at request time.
