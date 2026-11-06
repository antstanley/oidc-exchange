---
title: "API Reference"
description: "HTTP endpoints for token exchange, refresh, revocation, and user management."
---

oidc-exchange provides public endpoints for token operations and internal endpoints for user management. Response bodies use JSON where present. The token endpoints follow OAuth 2.0 conventions in RFC 6749. Revocation follows RFC 7009.

## Public endpoints

| Method | Path | Description |
|---|---|---|
| POST | `/token` | Token exchange and refresh |
| POST | `/revoke` | Token revocation |
| GET | `/keys` | JWKS (JSON Web Key Set) endpoint |
| GET | `/.well-known/openid-configuration` | OpenID Connect discovery document |
| GET | `/health` | Health check |
| POST | `/nonce` | Mint a single-use nonce for the direct ID-token grant (only when `[grants] id_token = true`) |

### POST /token (authorization code exchange)

Exchange an authorization code from an identity provider for access and refresh tokens.

**Request:**

```
POST /token
Content-Type: application/x-www-form-urlencoded

grant_type=authorization_code
&code=AUTH_CODE_FROM_PROVIDER
&provider=google
&redirect_uri=https://app.example.com/callback
```

| Parameter | Required | Description |
|---|---|---|
| `grant_type` | Yes | Must be `authorization_code` |
| `code` | Yes | Authorization code from the identity provider |
| `provider` | Yes | Provider name as configured (e.g., `google`, `apple`) |
| `redirect_uri` | Yes | The redirect URI used in the original authorization request |

**Response (200 OK):**

```json
{
  "access_token": "eyJhbGciOiJFZERTQSIsInR5cCI6IkpXVCIsImtpZCI6ImtleS0xIn0...",
  "refresh_token": "dGhpcyBpcyBhIHJlZnJlc2ggdG9rZW4...",
  "token_type": "Bearer",
  "expires_in": 900
}
```

The `access_token` is a signed JSON Web Token (JWT). Its lifetime defaults to 15 minutes. The `refresh_token` is a random 256-bit value encoded with base64url. Its lifetime defaults to 30 days. The service stores only its SHA-256 hash. You can configure both lifetimes.

### POST /token (refresh)

Use a refresh token to obtain a new access token without signing in again.

**Request:**

```
POST /token
Content-Type: application/x-www-form-urlencoded

grant_type=refresh_token
&refresh_token=dGhpcyBpcyBhIHJlZnJlc2ggdG9rZW4...
```

| Parameter | Required | Description |
|---|---|---|
| `grant_type` | Yes | Must be `refresh_token` |
| `refresh_token` | Yes | The refresh token from a previous exchange |

**Response (200 OK):**

```json
{
  "access_token": "eyJhbGciOiJFZERTQSIsInR5cCI6IkpXVCIsImtpZCI6ImtleS0xIn0...",
  "refresh_token": "bmV3LXJvdGF0ZWQtcmVmcmVzaC10b2tlbg...",
  "token_type": "Bearer",
  "expires_in": 900
}
```

By default (`[token] refresh_rotation = true`), each refresh rotates the token: the response carries a new `refresh_token`, the presented token is retired, and re-presenting a retired token is rejected as reuse (a short grace window covers a retried request). Setting `[token] refresh_rotation = false` is the only opt-out; it returns no new `refresh_token` and keeps the presented token valid until it expires or is revoked.

### POST /token (direct ID-token assertion)

Opt-in grant, served only when `[grants] id_token = true`. Exchange a provider ID token you already hold for access and refresh tokens, without an authorization code.

**Request:**

```
POST /token
Content-Type: application/x-www-form-urlencoded

grant_type=id_token
&provider=google
&id_token=PROVIDER_ID_TOKEN
&provider_access_token=PROVIDER_ACCESS_TOKEN
```

| Parameter | Required | Description |
|---|---|---|
| `grant_type` | Yes | Must be `id_token` |
| `provider` | Yes | Provider name as configured |
| `id_token` | Yes | The provider ID token to exchange |
| `provider_access_token` | No | Provider access token co-issued with the ID token, used only to verify its `at_hash` binding |

When the grant is disabled (the default), any request carrying an `id_token` field is rejected with `unsupported_grant_type`.

### POST /nonce

Mints a single-use nonce for the direct ID-token grant. Mounted only when `[grants] id_token = true`; it takes no request body.

**Response (200 OK):**

```json
{
  "nonce": "9c8b...",
  "expires_in": 600
}
```

### POST /revoke

Revoke a token as defined by RFC 7009. The endpoint returns `200 OK` even if the token is unknown or already revoked.

**Request:**

```
POST /revoke
Content-Type: application/x-www-form-urlencoded

token=dGhpcyBpcyBhIHJlZnJlc2ggdG9rZW4...
&token_type_hint=refresh_token
```

| Parameter | Required | Description |
|---|---|---|
| `token` | Yes | The token to revoke |
| `token_type_hint` | No | `refresh_token` or `access_token`. If a refresh token is revoked, only that session is invalidated. If an access token is revoked, only the single session (family) named by the token's `sid` claim is invalidated, not all of the user's sessions. |

**Response (200 OK):** Empty body.

### GET /keys

The endpoint returns the JSON Web Key Set (JWKS) containing public keys for verifying access token signatures. Your API uses these keys to validate tokens.

**Response (200 OK):**

```json
{
  "keys": [
    {
      "kty": "OKP",
      "crv": "Ed25519",
      "alg": "EdDSA",
      "use": "sig",
      "kid": "key-1",
      "x": "..."
    }
  ]
}
```

### GET /.well-known/openid-configuration

Returns the standard OpenID Connect discovery document.

**Response (200 OK):**

```json
{
  "issuer": "https://auth.example.com",
  "jwks_uri": "https://auth.example.com/keys",
  "token_endpoint": "https://auth.example.com/token",
  "revocation_endpoint": "https://auth.example.com/revoke",
  "grant_types_supported": ["authorization_code", "refresh_token"],
  "response_types_supported": ["code"],
  "subject_types_supported": ["public"],
  "id_token_signing_alg_values_supported": ["EdDSA"]
}
```

The `id_token_signing_alg_values_supported` field contains the configured key manager's algorithm.

### GET /health

The endpoint returns `200 OK` if the service is running. It does not require authentication.

## Internal endpoints

Internal endpoints provide user CRUD and claims management. They are served on a dedicated admin listener (default `127.0.0.1:8081`), and are mounted only when `server.role` is `admin` or `all` and `internal_api.enabled = true`; with either condition unmet, no `/internal/*` routes exist. All internal routes require authentication.

### Authentication

Internal routes sit behind an operator-authentication gate that supports three mechanisms, tried in the order listed in `internal_api.auth_methods`:

- `operator_token` (recommended): an operator JWT verified against this service's own key manager, carrying the configured audience and required claim.
- `mtls`: the client-certificate subject asserted by a TLS-terminating proxy via a trusted header.
- `shared_secret` (legacy): a static secret presented as `Authorization: Bearer <shared_secret>` and compared in constant time.

A successful attempt attaches the authenticated operator principal to the request; a failed attempt counts against a per-peer lockout that answers with `429` and a `Retry-After` header once exhausted.

Example using the legacy shared-secret mechanism:

```toml
[internal_api]
enabled = true
auth_methods = ["shared_secret"]
shared_secret = "${INTERNAL_API_SECRET}"
```

### Internal routes

| Method | Path | Description |
|---|---|---|
| GET | `/internal/stats` | Aggregate user and session counts |
| POST | `/internal/sessions/cleanup` | Sweep expired sessions; returns `{ "deleted": <count> }` |
| GET | `/internal/users` | List users (query: `cursor`, `limit`) |
| POST | `/internal/users` | Create a user |
| GET | `/internal/users/{id}` | Get a user by ID |
| PATCH | `/internal/users/{id}` | Update a user |
| DELETE | `/internal/users/{id}` | Soft-delete a user (revokes all sessions) |
| GET | `/internal/users/{id}/sessions` | List the user's live sessions, newest first, at most 50 (404 for an unknown or soft-deleted user) |
| DELETE | `/internal/users/{id}/sessions` | Revoke all of the user's live sessions; returns `{ "sessions_revoked": <n> }` (404 for an unknown or soft-deleted user) |
| DELETE | `/internal/users/{id}/sessions/{sessionId}` | Revoke one live session by the `ses_…` id the list returned; `204` (404 for an unknown or soft-deleted user, or an id that is not one of that user's live sessions) |
| GET | `/internal/users/{id}/claims` | Get a user's private claims |
| PUT | `/internal/users/{id}/claims` | Replace all of a user's private claims |
| PATCH | `/internal/users/{id}/claims` | Merge into a user's private claims |
| DELETE | `/internal/users/{id}/claims` | Clear all of a user's private claims |

### GET /internal/users/{id}/sessions

Returns `{ "sessions": [...] }`, one entry per live refresh-token family. Each entry has an
opaque `id` (`ses_…`, stable across rotation), `provider`, `user_agent`, `ip_address`,
`created_at`, `expires_at`, and a `location` (`city`, `region`, `country`) that is omitted
when none was recorded. Refresh-token hashes, device ids, and family ids are never returned.
See [Session location](/guides/configuration/#session-location) for how locations are recorded.

### DELETE /internal/users/{id}/sessions/{sessionId}

Revokes the one live session (refresh-token family) whose opaque `ses_…` id
`GET /internal/users/{id}/sessions` returned, retired refresh-token records included, and
answers `204`. The id is resolved among that user's own listed sessions, so an id belonging to
another user, an unknown id, and an already-revoked id are all `404`. Repeating the call after
success is `404`.

### DELETE /internal/users/{id}/sessions

Revokes every live session of the user and answers `200 { "sessions_revoked": <n> }`. The user
stays active. Repeating the call returns `{ "sessions_revoked": 0 }`. If the user has more than 50
live sessions, this is the way to clear the ones beyond the list window.

Both routes record an operator-attributed audit event (`SessionRevoked` with
`detail.session_id`; `AllSessionsRevoked` with `detail.sessions_revoked`). Revocation ends
refresh-token families only: access tokens issued before the call stay valid until they expire,
exactly as with `POST /revoke`.

### POST /internal/users

Create a user. The service generates the user ID with a `usr_` prefix and a Universally Unique Lexicographically Sortable Identifier (ULID).

**Request:**

```json
{
  "external_id": "google-oauth2|123456",
  "provider": "google",
  "email": "user@example.com",
  "display_name": "Jane Doe"
}
```

### GET /internal/users/{id}

Returns the full user object including metadata and claims.

### PATCH /internal/users/{id}

Update user fields. Only provided fields are modified.

**Request:**

```json
{
  "display_name": "Jane Smith",
  "status": "suspended",
  "metadata": {
    "role": "admin"
  }
}
```

### DELETE /internal/users/{id}

Soft-deletes the user (sets status to `Deleted`) and revokes all active sessions.

### Claims endpoints

When issuing an access token, the service combines `[token.custom_claims]` with the user's claims. User claims override configuration claims with the same key.

- **PUT** `/internal/users/{id}/claims` replaces the entire claims map.
- **PATCH** `/internal/users/{id}/claims` merges new claims into existing ones.
- **DELETE** `/internal/users/{id}/claims` clears all per-user claims.

**Example (PUT):**

```json
{
  "role": "admin",
  "tier": "enterprise"
}
```

## Error response format

All error responses follow the OAuth 2.0 error format (RFC 6749 Section 5.2):

```json
{
  "error": "invalid_grant",
  "error_description": "Authorization code has expired"
}
```

### Error codes

| Error | HTTP Status | Cause |
|---|---|---|
| `invalid_grant` | 400 | Expired or invalid authorization code or refresh token |
| `invalid_request` | 400 | Missing required parameter or unknown provider |
| `unsupported_grant_type` | 400 | Unknown or empty `grant_type`, or an `id_token` field when the direct grant is disabled |
| `invalid_token` | 401 | Malformed or expired token |
| `unauthorized` | 401 | Missing or invalid authentication |
| `access_denied` | 403 | Registration denied (domain not allowed, existing_users_only mode, or user suspended) |
| `slow_down` | 429 | Rate limit exceeded; retry after the delay in the `Retry-After` header |
| `server_error` | 500/502/504 | Internal failure, provider error, or provider timeout |

For `server_error` responses, the service logs details internally and returns a generic message to the client.
