# Provider System

**Status:** Implemented · **Date:** 2026-08-23 · **Owner:** Ant Stanley · **Scope:** crates/adapters/oidc, crates/providers

Identity providers implement the [`IdentityProvider`](02-ports-and-adapters.md) port. The
service keeps them in a `HashMap<String, Box<dyn IdentityProvider>>` keyed by the config
section name and looks one up per `/token` request by the `provider` field.

## Tiers

The design accommodates three tiers; two are implemented.

**Tier 1 — standard OIDC (config only).** `adapters/oidc::OidcProvider` handles any
spec-compliant provider (e.g. Google) entirely from config:

```toml
[providers.google]
adapter = "oidc"
issuer = "https://accounts.google.com"
client_id = "${GOOGLE_CLIENT_ID}"
client_secret = "${GOOGLE_CLIENT_SECRET}"
scopes = ["openid", "email", "profile"]
```

`from_config` discovers the `token_endpoint`, `jwks_uri`, and `revocation_endpoint` from the
issuer's `.well-known/openid-configuration` when they are not given. Every endpoint —
configured or discovered — is an `https` URL; the config types make any other scheme
unrepresentable, and discovery rejects a response whose HTTP status is not a success before it
parses the body. Adding a Tier 1 provider is a new config block — no code.

**Tier 2 — OIDC with quirks (custom module).** `providers/apple::AppleProvider`:

```toml
[providers.apple]
adapter = "apple"
client_id = "com.example.app"      # Services ID
team_id = "${APPLE_TEAM_ID}"
key_id = "${APPLE_KEY_ID}"
private_key_path = "/secrets/apple.p8"
```

Apple is mostly OIDC but requires a freshly signed **ES256 client secret JWT** for each token
endpoint call (`ClientSecretClaims { iss: team_id, sub: client_id, aud, iat, exp }`, ~5-minute
lifetime, signed with the `.p8` key). `generate_client_secret` returns that assertion as
`Secret<String>`, so it can be posted but not formatted. `revoke_token` sends the assertion
alongside the token being revoked and renders any non-2xx response through
`shared::upstream::error_detail`. It reuses the shared `JwksCache` for the standard ID-token
validation parts.

Apple's ID tokens sometimes carry `email_verified` (and `is_private_email`) as the JSON
strings `"true"`/`"false"` rather than booleans. The Apple provider coerces bool-or-string
values when mapping to `IdentityClaims.email_verified` and
`IdentityClaims.is_private_email`, so the registration domain allowlist (which requires
`email_verified == Some(true)`) works for Apple sign-ins. `is_private_email` is a
first-class `Option<bool>` field on `IdentityClaims`, populated only by the Apple
provider; the generic OIDC provider leaves it `None`. The same `https` endpoint constraint
applies to Apple's optional `token_endpoint`, `jwks_uri`, and `revocation_endpoint` overrides,
which take the shared `HttpsUrl` type rather than repeating the check.

**Tier 3 — non-OIDC (e.g. atproto).** *Not implemented.* The `IdentityProvider` doc comment
and several config/example files name `atproto`, but no `AtprotoProvider` exists in the
codebase. Treat any atproto reference as aspirational until a change spec lands it.

## OidcProvider behaviour (`adapters/oidc`)

- `exchange_code` delegates to `shared::token_endpoint::exchange_code` (form-encoded
  `authorization_code` POST with client credentials). A non-2xx upstream response yields a
  detail built by `shared::upstream::error_detail`, never the raw body.
- `validate_id_token` decodes the JWT header, fetches the issuer's JWKS through the cached
  `JwksCache`, and validates the signature using the **algorithm from the JWK** (not the
  untrusted header), returning `IdentityClaims`. Validation requires the `exp`, `iss`,
  and `aud` claims to be **present** (`set_required_spec_claims`) and to match the
  configured issuer and `client_id`; `nbf` is validated when present. A token missing
  `iss` or `aud` — e.g. a provider access token presented as an ID token — is rejected.
  The returned `IdentityClaims` carries `signing_alg` — the algorithm the JWK actually
  verified with, not the header's — so the core's `at_hash` check can select the matching
  digest without re-deciding the algorithm. Both validators report it; neither performs
  any replay or binding check itself.
- When the matched JWK carries no `alg`, the algorithm is inferred from the key type:
  `kty: EC` by `crv` (P-256 → ES256, P-384 → ES384), `kty: OKP` → EdDSA, `kty: RSA` →
  RS256. Any other alg-less key is rejected. (Azure-AD-style JWKS omit `alg`.)
- `revoke_token` POSTs to the discovered revocation endpoint with the client id. A non-2xx
  response is read with `shared::http::read_bounded` and rendered through
  `shared::upstream::error_detail`, so an intermediary that echoes the submitted form cannot
  put the token being revoked into the error log.

## Provider registry

The bootstrap builds the registry from every `[providers.*]` block whose `adapter` is
recognised:

```
"oidc"  → OidcProvider::from_config
"apple" → AppleProvider::from_config
other   → error (unknown adapter)
```

For roles that do not serve `/token` (`admin`), the registry is not built. At request time an
unrecognised `provider` value yields `UnknownProvider` → HTTP 400 `invalid_request`.

## Assumptions and open questions

### Assumptions

- Provider issuers expose a standard `.well-known/openid-configuration`; where they do not,
  the endpoint fields must be set explicitly in config.
- The JWKS cache TTL (default 1h) is short enough to pick up upstream key rotation without
  manual intervention.

### Decisions

- *Algorithm from the JWK.* **ID-token validation uses the signing algorithm declared by the
  matched JWK, not the token header.** Closes the `alg`-confusion class of attacks.
- *Replay binding is above the provider boundary.* **No `IdentityProvider` implementation
  checks `nonce`, `azp`, `at_hash` or one-time use; `AppService::exchange` does, for all of
  them.** Two independent validators omitted the same four controls; adding them twice
  would leave a third implementation free to omit them again. This is consistent with the
  direction in `hardening/proposals/provider-response-boundary.md`: whether the two
  validators later consolidate behind a shared `ProviderTransport`/`VerificationKeySet` or
  collapse into one profile-driven validator, the binding does not move. `signing_alg` is
  the same algorithm-as-data that a `VerificationKeySet` would carry, surfaced early.
- *Apple as a separate crate.* **The Apple provider lives in `crates/providers`, not
  `crates/adapters`.** Provider-specific protocol logic (per-request client JWT) is kept apart
  from infrastructure adapters.
- *Provider keyed by config name.* **The `provider` request field maps to the `[providers.X]`
  section name, not an issuer URL.** Clients reference a stable short name; the issuer can
  change in config without changing clients.
- *Required spec claims.* **ID-token validation requires `exp`, `iss`, and `aud` to be
  present, not merely correct-when-present.** Closes the cross-token-type confusion class
  (e.g. Keycloak realm access tokens omit `aud`).

### Open questions

- atproto (Tier 3: PAR, DPoP, DID resolution) is documented but unimplemented; it needs a
  change spec and a new `providers/atproto` module before any surface advertises it.
