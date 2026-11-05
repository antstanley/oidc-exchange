# Provider System

**Status:** Implemented · **Date:** 2026-08-31 · **Owner:** Ant Stanley · **Scope:** crates/adapters/oidc, crates/providers

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
endpoint_origins = ["https://oauth2.googleapis.com", "https://www.googleapis.com"]
```

`from_config` discovers the `token_endpoint`, `jwks_uri`, and `revocation_endpoint` from the
issuer's `.well-known/openid-configuration` when they are not given. Every endpoint —
configured or discovered — is an `https` URL; the config types make any other scheme
unrepresentable, and discovery rejects a response whose HTTP status is not a success before
it parses the body. Every endpoint a discovery document supplies must also have an origin in
the provider's **pinned endpoint-origin set**: the issuer's own origin, plus the origin of
any endpoint the operator configured explicitly, plus every origin listed in
`endpoint_origins`. The set is fixed at config load, so a discovery document may confirm
which origins this service talks to but can never widen them — a compromised or hostile
document cannot relocate the verification-key source or the destination the client secret is
posted to. Cross-origin endpoints are ordinary, not exceptional: Google publishes its
`token_endpoint` and `revocation_endpoint` on `oauth2.googleapis.com` and its `jwks_uri` on
`www.googleapis.com`, none of which is the issuer's origin, which is why the set is declared
rather than derived. The check ships in warning mode for one release — an undeclared origin
logs a structured warning and the deployment is served unchanged — and rejecting undeclared
origins (`Warn` → `Enforce`) is a separate future release-owner decision made after that
warning window, not part of this change. Adding a Tier 1 provider is a config block — no
code. Two optional keys govern how the adapter derives `IdentityClaims.email_verified` for
providers that do not emit the standard claim; see
[Email-verification overrides](#email-verification-overrides).

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
alongside the token being revoked through the shared transport and renders any non-2xx
response through `shared::upstream::error_detail`. It reuses the shared `JwksCache` and the
shared `VerificationKeySet` for the standard ID-token validation parts, constructing the key
set with the admitted-algorithm set `{RS256, ES256}` — the two algorithms Apple's own
validator has always accepted. Its optional `token_endpoint`, `jwks_uri`, and
`revocation_endpoint` overrides are pinned the same way as a Tier 1 provider's: the defaults
are all on `appleid.apple.com`, so an override onto another origin must be declared in
`endpoint_origins`. The issuer stays pinned to the `https://appleid.apple.com` constant
regardless.

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
- `validate_id_token` decodes the JWT header for its `kid`, obtains the provider's
  `VerificationKeySet` through the cached `JwksCache`, and looks the `kid` up in it. The
  resolved `VerificationKey` carries the algorithm it will verify with, so the validation is
  configured from the **key set** rather than from a per-provider `alg` match and never from
  the untrusted header. A `kid` that matches only an ineligible entry is a miss: it takes the
  forced-refetch branch and then fails closed. Validation requires the `exp`, `iss`, and
  `aud` claims to be **present** (`set_required_spec_claims`) and to match the configured
  issuer and `client_id`; `nbf` is validated when present. A token missing `iss` or `aud` —
  e.g. a provider access token presented as an ID token — is rejected. The returned
  `IdentityClaims` carries `signing_alg` — the algorithm the resolved key actually verified
  with, not the header's — so the core's `at_hash` check can select the matching digest
  without re-deciding the algorithm. Both validators report it; neither performs any replay
  or binding check itself. The `email_verified` the returned claims carry is derived per
  the provider's configured email-verification mode (see
  [Email-verification overrides](#email-verification-overrides)); an explicit
  `email_verified` claim always passes through, bool-or-string coerced.
- Eligibility and algorithm are decided together, in the key set's constructor. An entry
  whose `use` is present and is not `"sig"`, or whose `key_ops` is present and omits
  `"verify"`, is not a candidate. An entry declaring an `alg` outside the provider's
  admitted set is dropped rather than falling through to inference, so a key published as
  `alg: "RSA-OAEP"` is rejected instead of being resolved to `RS256` from its key type.
  Inference applies only when `alg` is genuinely absent (Azure-AD-style JWKS omit it) and is
  restricted to the key shapes it can decide: `kty: RSA` → RS256, `kty: EC` by `crv`
  (P-256 → ES256, P-384 → ES384), `kty: OKP` with `crv: Ed25519` → EdDSA. Any other alg-less
  key is rejected, so an OKP key on a curve that is not a signature curve has no arm to land
  in. A resolved algorithm must agree with the entry's `kty`/`crv` before a decoding key is
  built.
- `revoke_token` POSTs to the discovered revocation endpoint with the client id, through the
  shared transport. A non-2xx response is read under the shared ceiling and rendered through
  `shared::upstream::error_detail`, so an intermediary that echoes the submitted form cannot
  put the token being revoked into the error log.

## Email-verification overrides

The registration policy ([03-service-flows.md](03-service-flows.md)) accepts an email
claim only when `IdentityClaims.email_verified == Some(true)`. The generic adapter
derives that field per provider, in `validate_id_token`, from one of three configured
modes:

| Mode | Config | Derivation when the token's own `email_verified` is absent |
|---|---|---|
| Standard (default) | *(neither key set)* | stays `None` — a provider that does not attest verification cannot pass registration policy |
| Mapped claim | `email_verified_claim = "<name>"` | read the named claim instead, bool-or-string coerced (`coerce_bool`); any other value is `None` |
| Trusted email | `trust_email_verified = true` | `Some(true)` iff the token carries a non-empty `email` string claim |

An explicit `email_verified` claim from the provider always passes through first — the
overrides fill absence, they never overturn the provider's own signal, so a token
carrying `email_verified: false` is unverified in every mode. Both keys are
oidc-adapter `extra` keys, lifted and validated in the server's
`provider_config_to_oidc` alongside `client_id` and `endpoint_origins`:
`email_verified_claim` must be a non-empty string of at most 64 characters,
`trust_email_verified` must be a TOML boolean (a set-but-non-boolean value is a config
error, never coerced), and setting both on one provider block is a config error. The
keys are meaningful only under `adapter = "oidc"`; the Apple adapter reads its own
config and always receives `email_verified` from Apple. A provider configured with a
non-default mode logs one structured startup warning at registry build naming the
provider and the mode — the toggle weakens an identity control and must be visible in
boot logs.

**Microsoft Entra ID (Azure AD) v2.0** is the motivating deployment: its id_tokens
carry `email` but no `email_verified` claim (scopes `openid email profile`), so under
the default mode every Entra sign-in fails registration policy. The supported recipe
maps Entra's optional `xms_edov` ("email domain owner verified") claim, which a tenant
administrator enables per app registration:

```toml
[providers.entra]
adapter = "oidc"
issuer = "https://login.microsoftonline.com/${ENTRA_TENANT_ID}/v2.0"
client_id = "${ENTRA_CLIENT_ID}"
client_secret = "${ENTRA_CLIENT_SECRET}"
scopes = ["openid", "email", "profile"]
email_verified_claim = "xms_edov"
```

`xms_edov` is off by default in Entra; tenants that cannot enable it may instead set
`trust_email_verified = true`, accepting that Entra's `email` claim is then trusted
without domain-ownership proof — Entra documents the claim as user-mutable in some
tenant configurations, which is exactly the input class `xms_edov` exists to attest.
Prefer the claim mapping wherever the tenant permits it.

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
- A provider's endpoint origins are known when it is configured. Providers do publish
  endpoints off the issuer's origin — Google is the shipped example — so the origins are
  declared in config rather than inferred from the issuer, and a provider that relocates an
  endpoint to a new origin needs a config change before discovery will accept it.
- The JWKS cache TTL (default 1h) is short enough to pick up upstream key rotation without
  manual intervention, and a key set served past its TTL while a refill is in flight is
  stale rather than untrusted.

### Decisions

- *Algorithm from the JWK, carried as data.* **ID-token validation uses the algorithm the
  resolved `VerificationKey` carries, decided in the key set's constructor, not the token
  header and not a per-provider match at the call site.** Closes the `alg`-confusion class,
  and removes the possibility of two providers deciding the same question differently.
- *Replay binding is above the provider boundary.* **No `IdentityProvider` implementation
  checks `nonce`, `azp`, `at_hash` or one-time use; `AppService::exchange` does, for all of
  them.** Two independent validators omitted the same four controls; adding them twice
  would leave a third implementation free to omit them again. The two validators now share
  the `ProviderTransport`/`VerificationKeySet` seam, and the binding still does not move.
  `signing_alg` is the algorithm-as-data the `VerificationKeySet` carries, surfaced to the
  core.
- *Apple as a separate crate.* **The Apple provider lives in `crates/providers`, not
  `crates/adapters`.** Provider-specific protocol logic (per-request client JWT) is kept apart
  from infrastructure adapters.
- *Provider keyed by config name.* **The `provider` request field maps to the `[providers.X]`
  section name, not an issuer URL.** Clients reference a stable short name; the issuer can
  change in config without changing clients.
- *Required spec claims.* **ID-token validation requires `exp`, `iss`, and `aud` to be
  present, not merely correct-when-present.** Closes the cross-token-type confusion class
  (e.g. Keycloak realm access tokens omit `aud`).
- *Key purpose is binding when declared, permissive when absent.* **`use` must be `"sig"`
  and `key_ops` must contain `"verify"` when either member is present; a JWK carrying
  neither is eligible.** RFC 7517 §4.2–4.3 make both optional and many identity providers
  omit them, so rejecting alg-less, use-less keys would break working deployments; treating
  a declared purpose as decoration is what let an encryption key verify an identity
  assertion.
- *One selector, per-provider admitted algorithms.* **Both providers use the same
  `VerificationKeySet`; the set of algorithms each admits is a parameter.** The generic
  adapter admits the nine JWS algorithms it always has and Apple admits `{RS256, ES256}`, so
  consolidating the selector neither widens Apple nor narrows the generic path. Whether the
  two validators are otherwise equivalent is threat-model contradiction C12, and it is
  answered by the cross-provider corpus rather than assumed by the merge.
- *Discovery may confirm origins, never widen them.* **A provider's endpoint-origin set is
  fixed at config load; a discovery-supplied endpoint outside it is rejected.** The RFC 8414
  issuer self-consistency check is a string comparison and constrains nothing about the
  endpoints the document goes on to name (threat-model contradiction C4). Pinning the set in
  config keeps the operator's declared intent authoritative over the provider's runtime
  assertion; enforcement follows a one-release warning window by explicit release-owner
  decision, so deployments relying on an undeclared cross-origin endpoint learn about it
  from a log line before it becomes an outage.
- *Verification is derived at the adapter boundary; the policy predicate is untouched.*
  **`registration_policy_reason` still requires `email_verified == Some(true)` on every
  path; the per-provider modes change only how the generic adapter derives that
  field.** The core cannot know which claim attests verification for which provider —
  that is provider dialect, and provider dialect lives in adapters (the Apple
  bool-or-string coercion set the precedent). Keeping the predicate closed also keeps
  the security-review surface one function.
- *Overrides fill absence, never overturn.* **An explicit `email_verified` claim wins
  over any configured override, in both directions.** A provider that says `false` has
  made a statement; a configuration that discarded it would turn a per-provider
  gap-filler into a general verification bypass.
- *Explicit, per-provider, default-strict.* **Both keys default off, are scoped to one
  provider block, and are mutually exclusive.** Trusting an unverified email is a
  security-sensitive weakening — a domain-allowlist entry could be satisfied by an
  attacker-authored address, and downstream consumers of `user.email` (user-sync
  webhooks, admin surfaces) inherit the trust — so it must be a deliberate, auditable,
  per-provider choice, never a global flag and never a side effect of a provider's
  claim shape.

### Open questions

- atproto (Tier 3: PAR, DPoP, DID resolution) is documented but unimplemented; it needs a
  change spec and a new `providers/atproto` module before any surface advertises it.
