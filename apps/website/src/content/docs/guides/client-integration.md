---
title: "Client Integration"
description: "How to integrate your application with oidc-exchange."
---

Your application handles sign-in with an OpenID Connect (OIDC) provider such as Google or Apple. OIDC is an identity protocol built on OAuth 2.0. Send the resulting authorization code to oidc-exchange. The service validates the provider's response and returns its own access and refresh tokens.

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

- `access_token` is a signed JSON Web Token (JWT) that expires after 15 minutes by default. Use it to authorize API requests.
- `refresh_token` is a random value that expires after 30 days by default. Store it securely and use it to obtain new access tokens.
- `expires_in` is the access token lifetime in seconds.

## Token refresh

When the access token expires, use the refresh token to obtain a new one without asking the user to sign in again:

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

Your API verifies access tokens with public keys from the JSON Web Key Set (JWKS) endpoint:

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

Configure a JWT library with JWKS support to use `https://auth.example.com/keys`. Configure key caching and refresh according to the library's documentation.

### Verification flow

1. Your API receives a request with `Authorization: Bearer <access_token>`.
2. Your JWT library reads the token header to find the key ID (`kid`).
3. It fetches and caches the matching public key from the JWKS endpoint.
4. It verifies the signature and checks the issuer (`iss`), audience (`aud`), and expiry (`exp`).
5. If validation succeeds, your API uses the token's claims, including the user ID (`sub`) and custom fields.

### Library examples

Most languages have JWT libraries with built-in JWKS support:

- **Node.js**: `jose` or `jsonwebtoken` with `jwks-rsa`.
- **Python**: `PyJWT` with `PyJWKClient`.
- **Go**: `go-jose` or `golang-jwt` with JWKS fetcher.
- **Rust**: `jsonwebtoken` with manual JWKS fetch, or `openidconnect`.

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

For client libraries that support OIDC discovery, configure the issuer URL. The library can then read the endpoint details from this document.
