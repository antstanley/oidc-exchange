# Admin UI — Overview

**Status:** Implemented · **Date:** 2026-06-24 · **Owner:** Ant Stanley · **Scope:** apps/admin-ui

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

`src/hooks.server.ts` is the gate. Public paths are `/login`, `/login/callback`, `/denied`;
every other route requires a valid `access_token` cookie. The hook decodes the JWT payload,
checks `exp`, and checks the admin claim (`hasAdminClaim`, default `role == "admin"`,
overridable via `REQUIRED_CLAIM`/`REQUIRED_VALUE`). Missing/expired token → redirect to
`/login`; valid token without the claim → `/denied`.

`src/lib/auth.ts` provides `getJwks` (fetch `${OIDC_EXCHANGE_URL}/keys`, 5-minute cache),
`decodeJwtPayload`, `hasAdminClaim`, and `isExpired`. The login page accepts a pasted access
token; `login/+page.server.ts` validates it and sets the httpOnly cookie.

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

`INTERNAL_API_URL`, `INTERNAL_API_SECRET`, `OIDC_EXCHANGE_URL` (for JWKS),
`REQUIRED_CLAIM` (default `role`), `REQUIRED_VALUE` (default `admin`).

## Assumptions and open questions

### Assumptions

- The admin UI runs adjacent to an `admin`- or `all`-role service and reaches `/internal/*` on
  a trusted network.
- The operator obtains an access token out of band (the same `/token` flow as any user) whose
  JWT carries the admin claim.

### Decisions

- *Secret stays server-side.* **The shared secret lives only in the SvelteKit server; the
  browser holds only the JWT cookie.** The browser never sees the internal-API credential.
- *Claim-gated, not RBAC.* **A single configurable claim check authorises operators.** Matches
  the service's non-goal of full RBAC; sufficient for an admin console.
- *Paste-token login.* **Login accepts a pasted access token rather than driving the OAuth
  flow.** The simplest server-rendered path to a verified admin session; a full provider
  redirect flow is not implemented.

### Open questions

- A `/sessions` list view and an in-UI audit-log viewer are not implemented (the service has no
  list-all-sessions endpoint); session management is per-user via revoke. A `login/callback`
  route is referenced by the hook's public-path list but the full provider-redirect flow is not
  built.
