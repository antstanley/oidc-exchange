# Admin UI — Overview

**Status:** Implemented · **Date:** 2026-08-23 · **Owner:** Ant Stanley · **Scope:** apps/admin-ui

A SvelteKit web console for operators to view stats and manage users, claims, and sessions. It
is a separate service that talks to the OIDC exchange service's internal API; it is not part of
the Rust binary.

> **Read first:** [.specs/architecture-principles.md](../../architecture-principles.md). The
> admin UI is a client of the service's internal routes
> ([service/specs/04-http-api.md](../../service/specs/04-http-api.md)); it adds no server
> capabilities of its own.

## Responsibilities

- Authenticate a human operator and confirm they hold the admin claim.
- Render a dashboard (`AdminStats`) and paginated user management backed by `/internal/*`.
- Keep the internal-API shared secret server-side; never expose it to the browser.

## Stack

SvelteKit with `@sveltejs/adapter-node` (server-rendered for auth), Tailwind CSS 4 via
`@tailwindcss/vite`, and LayerChart (built on D3) for dashboard charts.

## Authentication model

`src/hooks.server.ts` is the gate. Public paths are `/login` and `/denied`; every other route
requires a `__Host-admin_session` cookie carrying an access token whose JWS signature verifies
against the service's published JWKS. The gate reads no claim from an unverified token.

`src/lib/auth.ts` exposes one token-to-claims function, `verifyAccessToken(token)`. It fetches
`${OIDC_EXCHANGE_URL}/.well-known/openid-configuration`, requires HTTPS, checks the document's
issuer against `OIDC_EXCHANGE_ISSUER`, requires its `jwks_uri` to be same-origin HTTPS, and
verifies with the bounded local JWKS and advertised signing-algorithm allowlist. Discovery and
JWKS are cached for five minutes; failed loads are not cached, concurrent loads are coalesced,
and an initial verification failure triggers one bounded refresh for rotation. Fetches have a
five-second timeout and one-MiB response limit; discovery allows at most 16 algorithms and JWKS
at most 32 keys. Redirects, malformed JSON, upstream errors, unsupported `alg`/`typ`, absent or
unknown `kid`, bad signatures, and invalid required claims fail closed without logging tokens,
subjects, keys, or upstream bodies.

The verifier requires `exp`, `iss`, `aud`, `sub`, and `iat`, applies `nbf` when present, rejects a
future `iat`, requires configured issuer/audience equality, accepts only `JWT`, `at+jwt`, or an
absent type, and never accepts `none`. Only its frozen payload reaches `hasAdminClaim`, which
requires an exact string match against `REQUIRED_CLAIM`/`REQUIRED_VALUE` (defaults `role` and
`admin`). Arrays, numbers, objects, and missing claims are denied without coercion.

Absent or invalid protected-route sessions are cleared and redirected to `/login`; verified
non-admin sessions go to `/denied`. Login load and action verify before reading claims; invalid
submissions return 401 without persistence. Valid admin login sets `__Host-admin_session` with
`Secure`, `HttpOnly`, `SameSite=Strict`, `Path=/`, no `Domain`, and `maxAge` derived from verified
`exp`. Logout deletes the cookie with matching scope and attributes. There is no unverified JWT
decode helper.

## Pages (`src/routes`)

| Route | Purpose |
|---|---|
| `/login` | accept and validate an access token, set the auth cookie |
| `/logout` | clear the cookie |
| `/denied` | shown when the admin claim is absent |
| `(app)/` | dashboard: `getStats()` + first page of users |
| `(app)/users` | paginated user table (`listUsers(offset, limit)`) |
| `(app)/users/[id]` | user detail + claims editor |
| `(app)/settings` | placeholder for future settings |

## Internal API client (`src/lib/api.ts`)

Server-side fetch wrapper over the service internal routes, base `INTERNAL_API_URL` (default
`http://localhost:8081`), authenticated with `Authorization: Bearer ${INTERNAL_API_SECRET}`.
It calls `GET /internal/stats`, `GET/POST /internal/users`, `GET/PATCH/DELETE
/internal/users/:id`, and the four `/internal/users/:id/claims` verbs. `src/lib/types.ts`
mirrors the service `User` and `AdminStats`
([service/specs/01-domain-model.md](../../service/specs/01-domain-model.md)).

## Environment

`INTERNAL_API_URL`, `INTERNAL_API_SECRET`, `OIDC_EXCHANGE_URL` (HTTPS base for discovery and
JWKS), `OIDC_EXCHANGE_ISSUER` (exact accepted `iss`), `ADMIN_UI_AUDIENCE` (exact accepted `aud`),
`REQUIRED_CLAIM` (default `role`), and `REQUIRED_VALUE` (default `admin`).

The three OIDC values have no defaults and are required at module initialization. An unset value
or a service `[token] audience` that differs from `ADMIN_UI_AUDIENCE` admits no session.

## Assumptions and open questions

### Assumptions

- The admin UI runs adjacent to an `admin`- or `all`-role service and reaches `/internal/*` on
  a trusted network.
- The operator obtains an access token out of band (the same `/token` flow as any user) whose
  JWT carries the admin claim.
- `OIDC_EXCHANGE_URL` points to an `exchange`- or `all`-role instance where discovery and JWKS are
  mounted; an admin-only instance fails closed.
- The service publishes its live key algorithm and signs access tokens with that key manager. Its
  current single-key JWKS means rotation ends sessions signed by the previous key after refresh.
- Service `[token] audience` and console `ADMIN_UI_AUDIENCE` are configured identically.

### Decisions

- *Secret stays server-side.* **The shared secret lives only in the SvelteKit server; the
  browser holds only the JWT cookie.** The browser never sees the internal-API credential.
- *Claim-gated, not RBAC.* **A single configurable claim check authorises operators.** Matches
  the service's non-goal of full RBAC; sufficient for an admin console.
- *Paste-token login.* **Login accepts a pasted access token rather than driving the OAuth
  flow.** The simplest server-rendered path to a verified admin session; a full provider
  redirect flow is not implemented.
- *Verify, then read.* **`verifyAccessToken` is the only token-to-claims path and both gate and
  login use it.** Signature and required claims are checked before authorization.
- *Discovery with configured bindings.* **Discovery supplies JWKS and the live algorithm list,
  while issuer and audience remain mandatory console configuration.** A fetched document cannot
  define the identity it is meant to prove.
- *Exact claims and host cookie.* **Admin authorization compares one string exactly, and the
  unconditional Secure `__Host-` cookie uses strict same-site policy.** Browser-enforced host
  scope complements fail-closed verification.

### Open questions

- A `/sessions` list view and an in-UI audit-log viewer are not implemented (the service has no
  list-all-sessions endpoint); session management is per-user via revoke. The provider-redirect
  flow is not built, so no `login/callback` route is public. If built later, its cross-site return
  needs `SameSite=Lax` or a same-site landing hop.
- Whether this private package is deployed is unresolved. CI lints, format-checks, type-checks,
  and tests it, but does not build a release artifact.
