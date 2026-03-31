# Service Roles & Admin Web UI — Design Spec

**Date**: 2026-03-27

## Overview

Two changes: (1) allow the oidc-exchange binary to run in different roles for independent scaling, and (2) add a web-based admin UI for user and session management.

## Part 1: Service Roles

### Problem

The single binary serves both the public OIDC exchange API (high throughput, latency-sensitive) and the internal user management API (low throughput, admin-only). These have different scaling characteristics and security profiles. Running them as separate services allows independent scaling and tighter network isolation.

### Design

Add a `role` field to `[server]` config:

```toml
[server]
role = "all"  # "all" | "exchange" | "admin"
```

| Role | Routes mounted | Adapters required |
|------|---------------|-------------------|
| `exchange` | `/token`, `/revoke`, `/keys`, `/.well-known/*`, `/health` | user_repo, session_repo, keys, providers, audit |
| `admin` | `/internal/*`, `/health` | user_repo, session_repo, audit, user_sync |
| `all` | All of the above (default, backwards compatible) | All |

**Route building in main.rs:**

```rust
let mut app = Router::new();

if role == "exchange" || role == "all" {
    app = app.merge(routes::public_routes());
}
if role == "admin" || role == "all" {
    app = app.merge(routes::internal_routes(state.clone()));
}
// Health is always mounted
app = app.route("/health", get(health::health_handler));
```

**Adapter skipping:** When role is `exchange`, skip building `user_sync`. When role is `admin`, skip building `providers` and `keys` (key manager). Pass `None` or noop adapters for unused ports.

**Auth change for admin role:** The internal API currently uses shared-secret auth. This remains unchanged — the admin UI authenticates with the internal API using this secret as a backend-to-backend credential. The admin UI itself authenticates its human users via JWT role claims (see Part 2).

### Config examples

Exchange-only service:
```toml
[server]
role = "exchange"
port = 8080
```

Admin-only service:
```toml
[server]
role = "admin"
port = 8081

[internal_api]
enabled = true
shared_secret = "${INTERNAL_API_SECRET}"
```

## Part 2: Admin Web UI

### Problem

Managing users, sessions, and claims currently requires direct API calls with curl or scripts. A web UI makes this accessible to non-technical operators.

### Design

A standalone SvelteKit application in `apps/admin-ui/`. It connects to the oidc-exchange internal API as a client.

**Tech stack:**
- SvelteKit with Node adapter (server-rendered for auth)
- TailwindCSS 4 for styling
- LayerChart (built on D3) for dashboard charts
- Communicates with oidc-exchange internal API via server-side fetch

**Authentication model:**

The admin UI authenticates its human users using oidc-exchange's own tokens. The flow:

1. Admin user authenticates via the oidc-exchange `/token` endpoint (same as any user)
2. The admin's access token must contain a `role` claim with value `"admin"`
3. SvelteKit server-side hooks verify the JWT on each request by checking the JWKS endpoint
4. Once verified, the SvelteKit backend calls the internal API using the shared secret
5. The admin UI is a separate service — it never exposes the shared secret to the browser

The required claim name and value are configurable:

```toml
[admin_ui]
required_claim = "role"
required_value = "admin"
oidc_exchange_url = "http://localhost:8080"
internal_api_url = "http://localhost:8081"
internal_api_secret = "${INTERNAL_API_SECRET}"
```

**Pages:**

1. **Dashboard** (`/`)
   - Total users (active, suspended, deleted)
   - User registrations over time (line chart)
   - Active sessions count
   - Sessions created over time (line chart)
   - Recent activity feed (last 20 events from users table)

2. **Users** (`/users`)
   - Paginated table: ID, email, provider, status, created_at
   - Search by email or external ID
   - Filter by status (active/suspended/deleted) and provider
   - Click row → user detail page

3. **User Detail** (`/users/[id]`)
   - User fields (read-only: id, external_id, provider, created_at)
   - Editable fields: email, display_name, status (dropdown)
   - Metadata editor (JSON)
   - Claims editor (JSON key-value pairs with add/remove)
   - Sessions list for this user (with revoke buttons)
   - Danger zone: suspend, delete, revoke all sessions

4. **Sessions** (`/sessions`)
   - Note: The current internal API doesn't have a list-all-sessions endpoint. The dashboard will derive session metrics from the user data (sessions per user). Individual session management happens on the user detail page via `revoke_all_user_sessions`.

5. **Login** (`/login`)
   - "Sign in with Google" (or configured provider) button
   - Redirects through oidc-exchange OAuth flow
   - On callback, verifies the `role` claim
   - Denied page if claim is missing

**Data flow:**

```
Browser → SvelteKit Server → oidc-exchange internal API
  │                              (shared secret auth)
  │
  └── JWT cookie (httpOnly, secure)
       verified against oidc-exchange JWKS
```

**API endpoints needed:** The existing internal API covers all CRUD operations. No new Rust endpoints are required. The dashboard metrics are computed client-side from user list data (or by adding a simple count endpoint later).

### New internal API endpoint

Add one new endpoint to support the dashboard:

```
GET /internal/stats
```

Returns:
```json
{
  "users": { "total": 150, "active": 140, "suspended": 8, "deleted": 2 },
  "sessions": { "active": 320 }
}
```

This requires adding a `count_users_by_status()` method to `UserRepository` and a `count_active_sessions()` method to `SessionRepository`.

## Scope

### In scope
- `[server] role` config field with `all`/`exchange`/`admin` values
- Conditional route mounting and adapter construction in main.rs
- `/internal/stats` endpoint for dashboard
- Count methods on repository traits
- SvelteKit admin UI with login, dashboard, user management, claims editor
- LayerChart dashboard charts
- TailwindCSS styling

### Out of scope
- RBAC beyond a single admin claim check
- Audit log viewer in the UI (future)
- Real-time WebSocket updates
- User creation from the UI (admin creates users via API or they self-register)
