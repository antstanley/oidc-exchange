---
title: "API Reference"
description: "HTTP endpoints for token exchange, refresh, revocation, and user management."
---

oidc-exchange exposes public endpoints for token operations and internal endpoints for user management. All responses use JSON. The API follows OAuth 2.0 conventions (RFC 6749) for token endpoints and RFC 7009 for revocation.

## Public endpoints

| Method | Path | Description |
|---|---|---|
| POST | `/token` | Token exchange and refresh |
| POST | `/revoke` | Token revocation |
| GET | `/keys` | JWKS (JSON Web Key Set) endpoint |
| GET | `/.well-known/openid-configuration` | OpenID Connect discovery document |
| GET | `/health` | Health check |

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

The `access_token` is a signed JWT with configurable lifetime (default 15 minutes). The `refresh_token` is an opaque token (256-bit random, base64url-encoded) with a longer lifetime (default 30 days). Only the SHA-256 hash of the refresh token is stored server-side.

### POST /token (refresh)

Use a refresh token to obtain a new access token without re-authenticating.

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
  "token_type": "Bearer",
  "expires_in": 900
}
```

Refresh does not issue a new refresh token. The original refresh token remains valid until it expires or is revoked.

### POST /revoke

Revoke a token per RFC 7009. Always returns `200 OK`, even if the token is unknown or already revoked.

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
| `token_type_hint` | No | `refresh_token` or `access_token`. If a refresh token is revoked, only that session is invalidated. If an access token is revoked, all sessions for the user are revoked (since individual JWTs cannot be revoked). |

**Response (200 OK):** Empty body.

### GET /keys

Returns the JSON Web Key Set containing the public key(s) used to sign access tokens. Downstream services use this endpoint to verify token signatures.

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

The `id_token_signing_alg_values_supported` field is dynamically populated from the configured key manager's algorithm.

### GET /health

Returns `200 OK` if the service is operational. No authentication required.

## Internal endpoints

Internal endpoints provide user CRUD and claims management. All internal routes require authentication.

### Authentication

Internal routes are protected by a shared secret. Callers must include the secret in the `Authorization` header:

```
Authorization: Bearer <shared_secret>
```

The shared secret is configured via:

```toml
[internal_api]
auth_method = "shared_secret"
shared_secret = "${INTERNAL_API_SECRET}"
```

Middleware compares the provided secret using constant-time comparison.

### Internal routes

| Method | Path | Description |
|---|---|---|
| POST | `/internal/users` | Create a user |
| GET | `/internal/users/{id}` | Get a user by ID |
| PATCH | `/internal/users/{id}` | Update a user |
| DELETE | `/internal/users/{id}` | Soft-delete a user (revokes all sessions) |
| GET | `/internal/users/{id}/claims` | Get a user's private claims |
| PUT | `/internal/users/{id}/claims` | Replace all of a user's private claims |
| PATCH | `/internal/users/{id}/claims` | Merge into a user's private claims |
| DELETE | `/internal/users/{id}/claims` | Clear all of a user's private claims |

### POST /internal/users

Create a new user. The user ID is generated server-side (`usr_` prefix + ULID).

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

Per-user claims are merged on top of config-level `[token.custom_claims]` when issuing access tokens. Per-user claims take precedence over config claims with the same key.

- **PUT** `/internal/users/{id}/claims` --- replaces the entire claims map
- **PATCH** `/internal/users/{id}/claims` --- merges new claims into existing ones
- **DELETE** `/internal/users/{id}/claims` --- clears all per-user claims

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
| `invalid_token` | 401 | Malformed or expired token |
| `unauthorized` | 401 | Missing or invalid authentication |
| `access_denied` | 403 | Registration denied (domain not allowed, existing_users_only mode, or user suspended) |
| `server_error` | 500/502/504 | Internal failure, provider error, or provider timeout |

Internal details are never leaked to the client. `server_error` responses log the detail internally and return a generic message.
