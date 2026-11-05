---
title: "Identity Providers"
description: "Configure Google, Apple, and custom OIDC providers."
---

oidc-exchange supports providers that use OpenID Connect (OIDC), an identity protocol built on OAuth 2.0. Standard providers such as Google need only configuration. Providers with specific requirements, such as Apple, use dedicated modules. Support for the non-OIDC provider atproto is planned.

Register each provider in a `[providers.<name>]` configuration block. Clients pass `<name>` in the `provider` field of `POST /token` requests.

## Standard OIDC providers (config-only)

Add a provider that follows OpenID Connect with a configuration block. The `OidcProvider` adapter discovers endpoints, caches public keys from a JSON Web Key Set (JWKS), exchanges authorization codes, and validates identity (ID) tokens.

### Google example

```toml
[providers.google]
adapter = "oidc"
issuer = "https://accounts.google.com"
client_id = "${GOOGLE_CLIENT_ID}"
client_secret = "${GOOGLE_CLIENT_SECRET}"
scopes = ["openid", "email", "profile"]
endpoint_origins = ["https://oauth2.googleapis.com", "https://www.googleapis.com"]
```

At startup, the adapter reads `https://accounts.google.com/.well-known/openid-configuration`. This document provides the token endpoint, public key URL, and revocation endpoint. The adapter caches keys and refreshes them when their cache lifetime ends.

#### Why `endpoint_origins` is there

Google publishes its endpoints off its issuer's origin: the token and revocation endpoints live on `oauth2.googleapis.com` and the JWKS URI on `www.googleapis.com`. The service pins, per provider, the set of origins a discovery document is permitted to name: the issuer's own origin, plus the origins of any endpoint you configure explicitly, plus every origin listed in `endpoint_origins` (each a bare `https://host[:port]`). The set is fixed when configuration loads: discovery can confirm which origins this service talks to but can never widen them, so a compromised or hostile discovery document cannot relocate where verification keys are fetched from or where the client secret is posted. If your provider serves an endpoint from an origin that is not pinned, add it to `endpoint_origins`; while the check runs in warning mode (the shipped default) an undeclared origin logs a structured warning naming the endpoint, the observed origin, and the permitted set, and rejecting such origins outright becomes a later release's decision. See the [configuration reference](/guides/configuration/) for the exact syntax.

### Adding any standard OIDC provider

To add a new provider, create a `[providers.<name>]` block with the following fields:

| Field | Required | Description |
|---|---|---|
| `adapter` | Yes | Must be `"oidc"` for standard providers |
| `issuer` | Yes | The provider's issuer URL (used for OIDC discovery) |
| `client_id` | Yes | OAuth client ID from the provider |
| `client_secret` | Cond. | OAuth client secret (use `${VAR_NAME}` placeholder). Optional in config; required by providers whose token endpoint expects one |
| `scopes` | No | Defaults to `["openid"]`. Parsed but not sent on the back-channel code exchange (this service does not perform the authorization redirect) |
| `jwks_uri` | No | Override discovered JWKS URI |
| `token_endpoint` | No | Override discovered token endpoint |
| `revocation_endpoint` | No | Override discovered revocation endpoint |
| `endpoint_origins` | No | Extra origins (bare `https://host[:port]`) the provider's discovery document may name beyond the issuer's origin and configured-endpoint origins; defaults to empty |
| `email_verified_claim` | No | oidc adapter only. String, no default. Name of a claim to read (bool-or-string coerced) when the token has no `email_verified` claim of its own, e.g. Entra's `xms_edov`. Must be a non-empty string of at most 64 characters. Mutually exclusive with `trust_email_verified` |
| `trust_email_verified` | No | oidc adapter only. Boolean, defaults to `false`. When `true`, a token whose `email_verified` claim is absent counts as verified iff it carries a non-empty `email` string claim. An explicit `false` is identical to omitting the key. Mutually exclusive with `email_verified_claim` |

For most Tier 1 providers, `issuer`, `client_id`, and `client_secret` are the fields you set. Endpoint fields are populated from the issuer's `.well-known/openid-configuration` at startup. If provided in config, they override the discovered values. If you override an endpoint onto another host, its origin joins the pinned set automatically; use `endpoint_origins` for extra origins only the *discovered* document names.

The email-verification overrides apply only when the provider omits `email_verified`. An explicit `true` or `false` always takes precedence. Setting both overrides for one provider causes a startup configuration error. Each provider using an override logs a structured warning at startup to make the changed trust policy visible.

### Examples

**Microsoft Entra ID:**

```toml
[providers.entra]
adapter = "oidc"
issuer = "https://login.microsoftonline.com/${ENTRA_TENANT_ID}/v2.0"
client_id = "${ENTRA_CLIENT_ID}"
client_secret = "${ENTRA_CLIENT_SECRET}"
scopes = ["openid", "email", "profile"]
email_verified_claim = "xms_edov"
```

Entra v2.0 ID tokens include `email` but omit `email_verified`. Without an override, the service treats the email as unverified and denies sign-in. Set `email_verified_claim` to Entra's optional `xms_edov` claim, which means "email domain owner verified". The adapter reads it as a boolean or converts a string value when `email_verified` is absent. A tenant administrator must enable `xms_edov` on the app registration before Entra includes it.

If your tenant cannot enable `xms_edov`, remove `email_verified_claim` and set `trust_email_verified = true`. This treats a non-empty `email` claim as verified when `email_verified` is absent. Some Entra tenant configurations let users change their email claim. Prefer the `xms_edov` mapping when your tenant supports it.

**GitHub (via OIDC-compatible endpoint):**

```toml
[providers.github]
adapter = "oidc"
issuer = "https://token.actions.githubusercontent.com"
client_id = "${GITHUB_CLIENT_ID}"
client_secret = "${GITHUB_CLIENT_SECRET}"
scopes = ["openid", "email"]
```

## Apple Sign-In

Apple requires a signed JSON Web Token (JWT) as the `client_secret` for token requests. Sign this short-lived token with your private key using the ES256 algorithm and a P-256 key.

Provide your Apple credentials in the configuration. The `AppleProvider` module generates and signs the client JWT.

### Configuration

```toml
[providers.apple]
adapter = "apple"
client_id = "com.example.app"
team_id = "${APPLE_TEAM_ID}"
key_id = "${APPLE_KEY_ID}"
private_key_path = "/secrets/apple.p8"
```

| Field | Required | Description |
|---|---|---|
| `adapter` | Yes | Must be `"apple"` |
| `client_id` | Yes | Your Apple Services ID (e.g., `com.example.app`) |
| `team_id` | Yes | Your Apple Developer Team ID |
| `key_id` | Yes | The Key ID from your Apple Developer account |
| `private_key_path` | Yes | Path to the `.p8` private key file (P-256/ES256) |
| `endpoint_origins` | No | Extra origins (bare `https://host[:port]`) an override endpoint may use beyond `appleid.apple.com` |

Apple's endpoint overrides follow the same origin pinning as standard providers. The defaults all live on `appleid.apple.com`, so an override onto another origin must be declared in `endpoint_origins`; the issuer itself stays pinned to `https://appleid.apple.com` regardless.

### How the ES256 client JWT works

For each token endpoint call, the Apple provider follows these steps:

1. It loads the P-256 private key from `private_key_path`.
2. It creates a JWT with fields called claims. These include `iss` (team ID), `sub` (client ID), `aud` (`https://appleid.apple.com`), `iat` (issue time), and `exp` (expiry).
3. It signs the JWT with ES256 using the private key.
4. It sends the JWT as `client_secret` to Apple's token endpoint.

The rest of the flow (JWKS fetching, ID token validation, discovery) reuses the shared OIDC utilities from the adapters crate.

### Getting your Apple credentials

1. Go to the [Apple Developer Portal](https://developer.apple.com/account).
2. Create a Services ID under **Certificates, Identifiers & Profiles** (this is your `client_id`).
3. Create a Sign In with Apple key (this gives you a `.p8` file, a Key ID, and your Team ID).
4. The `.p8` file is a P-256 private key in PKCS#8 format.

## Provider config format reference

Every provider block follows this general structure:

```toml
[providers.<name>]
adapter = "<adapter_type>"    # "oidc" or "apple"  (atproto is planned)
# ... adapter-specific fields
```

The `<name>` is the identifier clients use in `POST /token` requests. When a request arrives with `provider=google`, the service looks up the `"google"` key in its provider registry. An unknown provider returns `400 Bad Request`.

The server creates providers from configuration at startup. It selects the provider by name for each request.
