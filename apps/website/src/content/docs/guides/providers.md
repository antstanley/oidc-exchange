---
title: "Identity Providers"
description: "Configure Google, Apple, and custom OIDC providers."
---

oidc-exchange supports three tiers of identity providers. Standard OIDC providers like Google are config-only --- no code required. Providers with non-standard behavior like Apple have dedicated modules. Each provider is registered as a `[providers.<name>]` block in the configuration file, and the name is what clients pass in the `provider` field of `POST /token` requests.

## Standard OIDC providers (config-only)

Any provider that follows the OpenID Connect specification can be added with just a configuration block. The generic `OidcProvider` adapter handles discovery, JWKS caching, code exchange, and ID token validation automatically.

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

At startup, the adapter fetches `https://accounts.google.com/.well-known/openid-configuration` to discover the token endpoint, JWKS URI, and revocation endpoint. JWKS keys are cached with TTL-based automatic refresh.

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

For most Tier 1 providers, `issuer`, `client_id`, and `client_secret` are the fields you set. Endpoint fields are populated from the issuer's `.well-known/openid-configuration` at startup. If provided in config, they override the discovered values. If you override an endpoint onto another host, its origin joins the pinned set automatically; use `endpoint_origins` for extra origins only the *discovered* document names.

### Examples

**Microsoft Entra ID:**

```toml
[providers.microsoft]
adapter = "oidc"
issuer = "https://login.microsoftonline.com/{tenant-id}/v2.0"
client_id = "${MICROSOFT_CLIENT_ID}"
client_secret = "${MICROSOFT_CLIENT_SECRET}"
scopes = ["openid", "email", "profile"]
```

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

Apple follows OIDC for most of the flow but has a significant quirk: instead of sending a static `client_secret` to the token endpoint, Apple requires you to generate a short-lived ES256 (P-256) JWT signed with your private key and send that as the client secret on every request.

The `AppleProvider` module handles this automatically. You provide your Apple credentials and the service generates the signed client JWT internally.

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

For each token endpoint call, the Apple provider:

1. Loads the P-256 private key from `private_key_path`
2. Constructs a JWT with claims: `iss` (team_id), `sub` (client_id), `aud` ("https://appleid.apple.com"), `iat`, `exp` (short-lived)
3. Signs the JWT with ES256 using the private key
4. Sends this JWT as the `client_secret` parameter to Apple's token endpoint

The rest of the flow (JWKS fetching, ID token validation, discovery) reuses the shared OIDC utilities from the adapters crate.

### Getting your Apple credentials

1. Go to the [Apple Developer Portal](https://developer.apple.com/account)
2. Create a Services ID under **Certificates, Identifiers & Profiles** --- this is your `client_id`
3. Create a Sign In with Apple key --- this gives you a `.p8` file, a Key ID, and your Team ID
4. The `.p8` file is a P-256 private key in PKCS#8 format

## Provider config format reference

Every provider block follows this general structure:

```toml
[providers.<name>]
adapter = "<adapter_type>"    # "oidc" or "apple"  (atproto is planned)
# ... adapter-specific fields
```

The `<name>` is the identifier clients use in `POST /token` requests. When a request arrives with `provider=google`, the service looks up the `"google"` key in its provider registry. An unknown provider returns `400 Bad Request`.

Providers are constructed at startup from configuration. The server builds a `HashMap<String, Box<dyn IdentityProvider>>` and resolves providers by name at request time.
