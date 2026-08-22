# OIDC Exchange Service — Overview

**Status:** Implemented · **Date:** 2026-08-21 · **Owner:** Ant Stanley · **Scope:** crates/*

The Rust service at the heart of `oidc-exchange`. It validates ID tokens from third-party
OIDC providers and exchanges them for self-issued, short-lived access tokens and long-lived
refresh tokens. This page is the entry point for the service spec set; read it first, then
the detail pages below.

> **Read first:** the global specs —
> [architecture-principles.md](../../architecture-principles.md) for the hexagonal layering,
> workspace layout, and dependency rules this spec assumes, and
> [development-guidelines.md](../../development-guidelines.md) for the toolchain and coding
> discipline.

## Problem

An application that authenticates users through Google, Apple, or another OIDC provider must
validate the provider's ID token and then issue its own tokens for internal API
authorization. Doing this correctly means handling provider quirks (Apple's per-request
client JWT), full claim verification (signature, `iss`, `aud`, `exp`), hashed and revocable
refresh tokens, registration policy, and an audit trail — all of which are easy to get
subtly wrong when rolled by hand.

## Goals

- Accept either an authorization `code` (exchanged at the provider) or an `id_token`
  directly, and validate the resulting ID token in full.
- Issue short-lived access-token JWTs (default 15m) signed by a pluggable key manager, and
  long-lived opaque refresh tokens (default 30d) stored only as SHA-256 hashes.
- Keep all infrastructure behind port traits so storage, signing, audit, and user-sync
  backends are selected from TOML at runtime.
- Enforce a registration policy: open or existing-users-only, with an optional email
  domain/subdomain allowlist.
- Expose an internal admin API for user CRUD, per-user claims, and aggregate stats.
- Run as an axum server or an AWS Lambda from one binary, optionally split by `role`.
- Emit structured audit events with syslog severities and a configurable blocking threshold,
  plus OpenTelemetry-style tracing.

## Non-goals

- Hosting login pages, managing passwords, or running a full OAuth 2.0 authorization server.
  Authentication is delegated entirely to upstream providers.
- Rate limiting (expected from an upstream gateway/proxy), config hot-reload (restart to
  apply), and a token introspection endpoint (downstream verifies via JWKS).
- Multi-tenancy, RBAC beyond a single admin claim check, or SCIM provisioning.

## System shape

```
client ──code/id_token──► POST /token ─► provider.validate_id_token
                                       ─► registration policy (mode + domain allowlist)
                                       ─► user lookup / create  (UserRepository)
                                       ─► store hashed refresh   (SessionRepository)
                                       ─► sign access JWT         (KeyManager)
                                       ─► audit event             (AuditLog)
                                       ◄─ { access_token, refresh_token, ... }

downstream API ──verify JWT──► GET /keys (JWKS)   GET /.well-known/openid-configuration
operator ──Bearer secret──► /internal/* (user CRUD, claims, stats)  ◄── admin-ui
```

## Detail pages

| Page | Covers |
|---|---|
| [01-domain-model.md](01-domain-model.md) | Entities, IDs, lifecycles: User, Session, AuditEvent, token/claims types |
| [02-ports-and-adapters.md](02-ports-and-adapters.md) | The six port traits and every adapter that implements them |
| [03-service-flows.md](03-service-flows.md) | Exchange, refresh, revoke, admin, custom-claims and audit-blocking logic |
| [04-http-api.md](04-http-api.md) | Routes, middleware, roles, bootstrap, error mapping |
| [05-provider-system.md](05-provider-system.md) | Provider tiers, the OIDC and Apple providers, the registry |
| [06-configuration.md](06-configuration.md) | The full `AppConfig`, loading order, and defaults |
| [07-telemetry-and-audit.md](07-telemetry-and-audit.md) | Tracing/telemetry and how it relates to the audit trail |
| [08-persistence.md](08-persistence.md) | DynamoDB single-table design and the SQL/embedded session schemas |
| [canonical-types.schema.json](canonical-types.schema.json) | JSON Schema for every service entity |

## Crate map

| Crate | Role |
|---|---|
| `crates/core` | Domain types, the six port traits, `AppService` orchestration, config, errors |
| `crates/adapters` | DynamoDB / Postgres / SQLite / LMDB / Valkey storage, KMS / local / noop keys, stdout / SQS / noop audit, standard OIDC provider, webhook sync, shared OIDC utilities |
| `crates/providers` | Apple identity provider |
| `crates/server` | axum routes, middleware, telemetry init, bootstrap (config + adapter wiring + Lambda detection) |
| `crates/test-utils` | In-memory mock implementations of all ports |

## Scope summary

| Area | In service | Notes |
|---|---|---|
| Token exchange (`code` and `id_token` grants) | Yes | `crates/core/src/service/exchange.rs` |
| Token refresh, revocation | Yes | reusable refresh tokens; no rotation |
| Registration policy (mode + domain allowlist) | Yes | wildcard `*.example.com` supported |
| Custom claims (config templates + per-user) | Yes | restricted template language |
| Internal admin API (users, claims, stats) | Yes | shared-secret auth |
| Service roles (`all`/`exchange`/`admin`) | Yes | conditional route + adapter wiring |
| Standard OIDC provider, Apple provider | Yes | `crates/adapters/oidc`, `crates/providers/apple` |
| atproto / non-OIDC provider | No | named in docs and the `IdentityProvider` doc comment only; no implementation exists |
| Rate limiting, key rotation, config hot-reload, introspection | No | out of scope (see Non-goals) |

## Assumptions and open questions

### Assumptions

- Downstream services verify access tokens themselves via the `/keys` JWKS endpoint; the
  service issues tokens but does not introspect them.
- A scheduler external to the service drives `SessionRepository::cleanup_expired_sessions`
  where the store does not expire rows itself (DynamoDB TTL handles this natively).

### Decisions

- *Two grant inputs, each explicitly declared.* **`/token` accepts both a provider `code` and
  a raw `id_token`, and the declared `grant_type` selects which.** Browser SDKs (Google
  Identity Services) can post the credential they already hold without a second server-side
  code exchange, while which grant runs stays something the caller declares rather than
  something inferred from the fields they happened to send.
- *Opaque, hashed, reusable refresh tokens.* **256-bit random, stored as a SHA-256 hash,
  valid until expiry or revocation.** Revocable and leak-resistant, and reusable refresh
  matches what client libraries expect.
- *Single `aud` string.* **The access token carries one audience string, not an array.**
  Multi-audience is not implemented.

### Open questions

- atproto support is referenced in user-facing docs and example provider names but is not
  implemented. It belongs in a change spec before any code or doc claims it as shipped.
