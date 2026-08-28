---
title: "Client Integration"
description: "How to integrate your application with oidc-exchange."
---

Regardless of how oidc-exchange is deployed, clients interact with it the same way. Your application handles the OAuth flow with the identity provider (Google, Apple, etc.) and sends the resulting authorization code to oidc-exchange. The service validates the code, issues your own tokens, and returns them to the client.

## Token exchange

Your client application completes the provider's OAuth flow and obtains an authorization code. Send this code to oidc-exchange to receive an access token and refresh token:

```bash
curl -X POST https://auth.example.com/token \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d "grant_type=authorization_code" \
  -d "code=AUTH_CODE_FROM_PROVIDER" \
  -d "provider=google" \
  -d "redirect_uri=https://app.example.com/callback"
```

**Response:**

```json
{
  "access_token": "eyJhbGciOiJFZERTQSIsInR5cCI6IkpXVCIsImtpZCI6ImtleS0xIn0...",
  "refresh_token": "dGhpcyBpcyBhIHJlZnJlc2ggdG9rZW4...",
  "token_type": "Bearer",
  "expires_in": 900
}
```

- `access_token` is a short-lived JWT (default 15 minutes) signed by oidc-exchange. Use this for API authorization.
- `refresh_token` is a long-lived opaque token (default 30 days). Store it securely and use it to obtain new access tokens.
- `expires_in` is the access token lifetime in seconds.

## Token refresh

When the access token expires, use the refresh token to obtain a new one without requiring the user to re-authenticate:

```bash
curl -X POST https://auth.example.com/token \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d "grant_type=refresh_token" \
  -d "refresh_token=dGhpcyBpcyBhIHJlZnJlc2ggdG9rZW4..."
```

**Response:**

```json
{
  "access_token": "eyJhbGciOiJFZERTQSIsInR5cCI6IkpXVCIsImtpZCI6ImtleS0xIn0...",
  "refresh_token": "bmV3IHJvdGF0ZWQgcmVmcmVzaCB0b2tlbg...",
  "token_type": "Bearer",
  "expires_in": 900
}
```

By default (`refresh_rotation = true`) the refresh grant rotates the refresh token: the response carries a new `refresh_token` and the presented one is retired. Replace your stored refresh token with the new value on every refresh, and discard the one you sent. With `refresh_rotation = false` the response omits `refresh_token` and the presented token stays valid until it expires or is explicitly revoked.

## Token revocation

To revoke a token (e.g., on user logout), send it to the revocation endpoint:

```bash
curl -X POST https://auth.example.com/revoke \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d "token=dGhpcyBpcyBhIHJlZnJlc2ggdG9rZW4..." \
  -d "token_type_hint=refresh_token"
```

The endpoint returns `200 OK` per RFC 7009 for any token state, even if the token is unknown or already revoked, and returns `503` only if the storage backend is unavailable. Revocation is scoped to a single session: revoking a refresh token invalidates its session, and revoking an access token invalidates only the one session it was minted for (identified by its `sid` claim), not every session for the user.

## JWKS verification

Downstream services (your API servers) verify access tokens by fetching the public key from the JWKS endpoint:

```bash
curl https://auth.example.com/keys
```

**Response:**

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

Most JWT libraries support JWKS URLs natively. Point your verification middleware at `https://auth.example.com/keys` and it will cache and rotate keys automatically.

### Verification flow

1. Your API receives a request with `Authorization: Bearer <access_token>`
2. Your JWT library decodes the token header to find the `kid` (Key ID)
3. It fetches (and caches) the public key from the JWKS endpoint
4. It verifies the token signature, `iss` (issuer), `aud` (audience), and `exp` (expiration)
5. If valid, your API trusts the claims in the token (`sub` for user ID, plus any custom claims)

### Library examples

Most languages have JWT libraries with built-in JWKS support:

- **Node.js**: `jose` or `jsonwebtoken` with `jwks-rsa`
- **Python**: `PyJWT` with `PyJWKClient`
- **Go**: `go-jose` or `golang-jwt` with JWKS fetcher
- **Rust**: `jsonwebtoken` with manual JWKS fetch, or `openidconnect`

## OpenID Connect discovery

The discovery endpoint returns a standard document describing the service's capabilities:

```bash
curl https://auth.example.com/.well-known/openid-configuration
```

**Response:**

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

OIDC-compatible client libraries can auto-configure themselves from this endpoint. Point your library at the issuer URL and it will discover all endpoints automatically.
