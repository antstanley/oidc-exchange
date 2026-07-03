# Change: Server error handling, request-span correlation, and graceful shutdown

**Status:** Merged · **Date:** 2026-07-01 · **Merged:** 2026-07-03 · **Owner:** Ant Stanley · **Target:** crates/server

Stop `/revoke` from reporting success on infrastructure failure, log the internal detail that
`server_error` responses currently drop, make request-id correlation real by opening a
per-request tracing span, and shut the server down gracefully on SIGTERM — draining
in-flight requests up to a hard deadline — with a configurable request timeout bounding
slow clients.

---

## Motivation

Four server-layer gaps. `/revoke` discards every error from the core service
(`let _ = state.service.revoke(...)`), so a `StoreError` still returns 200 — a client is told
a stolen token is dead while the session lives. RFC 7009's always-200 rule covers invalid and
unknown tokens, not backend failure; §2.2.1 explicitly permits 503. Separately, the error
mapper returns correctly generic bodies for 500/502/504 but silently drops the internal
detail, while [04-http-api.md](../service/specs/04-http-api.md) states `server_error`
responses log it — the page describes the target state, the code does not meet it.

Correlation and shutdown are similarly hollow. The request-id middleware calls
`tracing::Span::current().record("request_id", ...)`, a guaranteed no-op — no per-request
span exists and `record` only writes pre-declared fields — so only the response-header echo
works and the request-id correlation [07-telemetry-and-audit.md](../service/specs/07-telemetry-and-audit.md)
relies on is impossible. And `axum::serve` runs without `with_graceful_shutdown` or a timeout
layer: SIGTERM (ECS/K8s rollouts) aborts in-flight exchanges, and slow clients hold
connections indefinitely.

---

## Affected spec pages

| Canonical page                                                                                 | Nature of change                                                                                                                                                                                                                                                                                                             |
| ---------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [`.specs/service/specs/03-service-flows.md`](../service/specs/03-service-flows.md)             | Revocation section: replace "always succeeds toward the client" / "Any failure is swallowed" with prose that swallows token-state failures (200) but propagates session-repo/backend failures (503)                                                                                                                          |
| [`.specs/service/specs/04-http-api.md`](../service/specs/04-http-api.md)                       | Modify the `/revoke` route row and the _Always-200 revoke_ Decision; extend the request-id middleware entry with the per-request span and insert the request-timeout layer into the Middleware stack; add graceful shutdown to Bootstrap. The error-mapping "log the internal detail" sentence already describes the target state and needs only a request-id mention |
| [`.specs/service/specs/06-configuration.md`](../service/specs/06-configuration.md)             | `[server]` section: add the `request_timeout` key; Defaults summary: add `request_timeout` = `"30s"` to the server row                                                                                                                                                                                                       |
| [`.specs/service/specs/07-telemetry-and-audit.md`](../service/specs/07-telemetry-and-audit.md) | No text change — the existing request-id correlation claim becomes accurate once the span exists                                                                                                                                                                                                                             |
| [`.specs/development-guidelines.md`](../development-guidelines.md)                             | Narrow the revoke best-effort carve-out — the "two documented paths" wording under _Errors are data, not exceptions_ and AI-agent rule 3 — to token-verification failures only                                                                                                                                                |

The per-request span overlaps the tower OTEL request-span layer proposed in
[2026-06-24-complete_telemetry_exporters.md](2026-06-24-complete_telemetry_exporters.md);
see Decisions for sequencing.

---

## Proposed changes

### `.specs/service/specs/03-service-flows.md` → Revocation (`revoke.rs`) (Modify)

Replaces "`POST /revoke` (RFC 7009 — always succeeds toward the client)." and the sentence
"Any failure is swallowed (still returns 200), since individual access JWTs cannot be
revoked."

> `POST /revoke` (RFC 7009 — token-state failures still succeed toward the client; backend
> failures propagate).
>
> - `token_type_hint == "access_token"` → `verify_and_extract_sub(token)`: split the JWT,
>   base64url the signature, `keys.verify(signing_input, signature)`, and on success decode
>   the payload and read `sub`; then `revoke_all_user_sessions(sub)`. A token-verification
>   failure (malformed, unsigned, expired, or unknown token) is swallowed and still returns
>   200 — individual access JWTs cannot be revoked and RFC 7009 §2.2 forbids leaking whether
>   a token existed — but a session-repo error from `revoke_all_user_sessions` propagates,
>   and the server maps it to 503.
> - hint `refresh_token`, absent, or unknown → SHA-256 hex the token and
>   `revoke_session(hash)`. A missing session is `Ok` (idempotent delete, 200); a store
>   error propagates, and the server maps it to 503.

### `.specs/service/specs/04-http-api.md` → Routes → Public (Modify)

> | POST | `/revoke` | `revoke` | RFC 7009 revocation: 200 for invalid/unknown tokens, 503 on backend failure |

### `.specs/service/specs/04-http-api.md` → Middleware stack (Modify)

Rewrites entry 1 and inserts a new entry 2; the existing audit-context and catch-panic
entries renumber to 3 and 4 unchanged.

> 1. **Request ID** (`middleware/request_id.rs`) — reuse `X-Request-Id` or generate a UUIDv4;
>    open a per-request `info_span` carrying `request_id` so all downstream logs — including
>    the `server_error` detail log — inherit it; echo in the response header.
> 2. **Request timeout** (`tower_http::timeout::TimeoutLayer`) — abort any request that runs
>    longer than `server.request_timeout` (default `30s`) and respond `408`. Sits inside the
>    request-id layer, so a timeout response still carries the request id, and outside the
>    rest of the stack, so the bound covers the remaining middleware and the handler.

### `.specs/service/specs/04-http-api.md` → Bootstrap (Modify)

> 6. Detect runtime: `AWS_LAMBDA_RUNTIME_API` present → Lambda mode; otherwise bind
>    `server.host:server.port` and serve over hyper with graceful shutdown — SIGTERM or
>    ctrl-c stops accepting connections and drains in-flight requests for up to a 10 s hard
>    deadline, after which stragglers are aborted and the process exits. The middleware
>    stack's request-timeout layer bounds slow clients at `server.request_timeout`
>    (default 30 s).

### `.specs/service/specs/06-configuration.md` → Sections → `[server]` (Modify)

> `host` (`0.0.0.0`), `port` (`8080`), `issuer` (the `iss` claim / discovery issuer, default
> empty), `role` (`all` | `exchange` | `admin`, default `all`), `request_timeout` (humantime
> duration string like the token TTLs, default `"30s"`) — the per-request timeout the
> server's timeout layer enforces.

### `.specs/service/specs/06-configuration.md` → Defaults summary (Modify)

The server row of the defaults table gains `request_timeout`:

> | `server.host` / `port` / `role` / `request_timeout` | `0.0.0.0` / `8080` / `all` / `"30s"` |

### `.specs/service/specs/04-http-api.md` → Error mapping (Modify)

> `server_error` responses (500/502/504) log the internal detail via `tracing::error!` —
> inside the request span, so the log carries the request id — and return a generic message;
> infrastructure detail is never leaked to the client.

### `.specs/service/specs/04-http-api.md` → Decisions (Modify)

> - _200 for token state, 503 for infrastructure._ **`/revoke` returns 200 whether the token
>   was revoked, invalid, or unknown, and 503 when the backend fails.** RFC 7009 forbids
>   leaking whether a token existed (§2.2) but permits 503 when the server cannot handle the
>   request (§2.2.1); a client must never be told a live session is dead.

### `.specs/development-guidelines.md` → Errors are data, not exceptions (Modify)

Narrows the revoke half of the "two documented best-effort paths" carve-out to
token-verification failures:

> - **Every error is handled or explicitly propagated.** Swallowing an error is a bug — except
>   the two documented best-effort paths (user-sync notifications, and token-verification
>   failures in the RFC 7009 revoke response — backend and session-repo failures on `/revoke`
>   propagate and map to 503), which log via `tracing` and are called out in the spec.

### `.specs/development-guidelines.md` → Guidelines for AI agents (Modify)

Rule 3 narrows the same way:

> 3. **No silent error swallowing.** Every error is handled; every match on an enum is
>    exhaustive. The only best-effort paths are the two documented ones (user-sync, and
>    revoke's token-verification failures — never revoke's backend errors).

---

## Type changes

None.

---

## Implementation notes

1. `crates/server/src/routes/revoke.rs:20-28`: match the `revoke` result — `Ok` → 200,
   `Err` → `tracing::error!` the detail and return 503 with the standard error body. Requires
   `crates/core/src/service/revoke.rs:20`, `:28`, `:34` to propagate session-repo errors with
   `?` instead of `let _ =`; confirm the adapters treat a missing token as `Ok` (idempotent
   delete) so a propagated `Err` is genuinely infrastructural.
2. `crates/server/src/error.rs:88-106` (`map_domain_error`): `tracing::error!` the source
   error for the `ProviderError`/`ProviderTimeout`/`StoreError`-class arms before returning
   the generic body.
3. `crates/server/src/middleware/request_id.rs:18`: replace the no-op `record` — build
   `tracing::info_span!("request", request_id, method, path)` and run `next.run(request)`
   instrumented with it (`tracing::Instrument`).
4. `crates/server/src/main.rs:36-37`: `axum::serve(...).with_graceful_shutdown(...)` awaiting
   SIGTERM (`tokio::signal::unix`) and ctrl-c; wrap the post-signal drain in
   `tokio::time::timeout` with a 10 s constant, aborting stragglers when it expires. Add
   `tower_http::timeout::TimeoutLayer` to the stack at `crates/server/src/bootstrap.rs:134-136`
   as entry 2 of the outermost-first ordering (inside request-id, outside audit-context and
   catch-panic), driven by a new `request_timeout: String` field on `ServerConfig`
   (`crates/core/src/config.rs:25`), defaulting to `"30s"` and parsed as a humantime duration
   like the `[token]` TTLs.

---

## Merge plan

1. Apply each `Proposed changes` block to its canonical page — the service pages
   (`03-service-flows.md`, `04-http-api.md`, `06-configuration.md`) and the repo-global
   `development-guidelines.md` — and bump each page's `**Date:**`. In `04-http-api.md`'s
   middleware stack, the inserted request-timeout entry renumbers audit-context and
   catch-panic to 3 and 4.
2. No change to `07-telemetry-and-audit.md`; verify its correlation sentence still reads true.
3. No schema change.
4. Flip `**Status:**` to `Merged`, stamp `**Merged:**`, move to `.specs/changes/merged/`.
5. Update `.specs/README.md`.

---

## Assumptions and open questions

### Assumptions

- Deployment platforms deliver SIGTERM and allow a drain window (ECS default 30 s, K8s
  `terminationGracePeriodSeconds`) before SIGKILL.
- The 503 revoke body reuses the standard `{"error": ..., "error_description": ...}` shape.

### Decisions

- _503, not 500, for revoke backend failure._ **Infrastructure failure on `/revoke` maps to 503.** Matches RFC 7009 §2.2.1 and signals retryability to well-behaved clients.
- _Revoke best-effort means token state only._ **The swallow-and-200 behaviour on `/revoke`
  covers token-verification failures only; session-repo and backend failures propagate.**
  `03-service-flows.md` and the `development-guidelines.md` carve-out (error-swallowing rule
  and AI-agent rule 3) narrow to match — a best-effort path never masks infrastructure
  failure.
- _The timeout layer is part of the enumerated middleware stack._ **Request timeout is entry
  2 in the outermost-first ordering, inside the request-id layer and outside everything
  else.** A timeout response still carries the request id, and the bound covers the
  remaining middleware and the handler rather than being applied ad hoc outside the stack.
- _One span per request._ **The request span is created once, in the middleware stack.** If
  the tower OTEL layer from the telemetry change spec lands first, `request_id` becomes a
  field on that span rather than a second span.
- _Request timeout is configuration, not a constant._ **A `[server] request_timeout` key
  (humantime string, default `"30s"`) drives the timeout layer.** Deployments front different
  providers with different latency tails; 30 s is a sane floor, not a universal one.
- _Spans merge across change specs, never nest._ **Whichever of this change and
  [2026-06-24-complete_telemetry_exporters.md](2026-06-24-complete_telemetry_exporters.md)
  ships second folds its per-request span into the other's single span.** Nested duplicate
  spans would split the correlation fields across two contexts.
- _Shutdown has its own hard deadline._ **Graceful shutdown drains in-flight requests for at
  most 10 s (constant), then aborts stragglers and exits.** The service terminates
  deterministically instead of leaning on each platform's SIGKILL timing.

### Open questions

- (None at this stage.)
