# Change: Per-provider email-verification overrides for the generic OIDC adapter

**Status:** Merged · **Date:** 2026-08-31 · **Merged:** 2026-08-31 · **Owner:** Ant Stanley · **Target:** crates/adapters (oidc), crates/core (domain), crates/server (bootstrap) — service spec pages 01, 03, 05, 06 + sidecar

Give each `[providers.<name>]` block using the generic `oidc` adapter an explicit,
opt-in way to derive `IdentityClaims.email_verified` when the provider's id_tokens do not
carry the standard `email_verified` claim: either map a named alternative claim
(`email_verified_claim = "xms_edov"`, bool-or-string coerced) or trust a non-empty `email`
claim as verified (`trust_email_verified = true`). The default is the current strict
behaviour, an explicit `email_verified` claim from the provider always wins over either
override, and the core registration-policy predicate — `email_verified == Some(true)` on
every path — is deliberately untouched. Microsoft Entra ID (Azure AD) v2.0 is the
motivating upstream and gets a documented recipe (GitHub issue #48).

---

## Motivation

The generic `oidc` adapter cannot complete a real sign-in against Microsoft Entra ID
v2.0. The registration policy (`registration_policy_reason`,
`crates/core/src/service/exchange.rs:96-115`) unconditionally requires
`email_verified == Some(true)` — for new users (`exchange.rs:349-357`), existing users
(`exchange.rs:331-342`), and the JIT-race re-lookup (`exchange.rs:425-436`) alike — but an
Entra v2.0 id_token issued for scopes `openid email profile` carries `email` and **no
`email_verified` claim** (observed claim set: `aud, email, exp, iat, idp, iss, name, nbf,
oid, preferred_username, rh, sid, sub, tid, uti, ver`). The adapter maps the absent claim
to `None` (`crates/adapters/src/oidc/mod.rs:190`), the predicate refuses anything short of
`Some(true)`, and every Entra sign-in — including pre-registered users — terminates in
`403 access_denied` ("verified email required for registration"). No configuration
workaround exists in `@oidc-exchange/*` 0.4.0: the adapter exposes no knob
(`OidcProviderConfig`, `crates/core/src/domain/provider.rs:8-35`), and the website's own
Entra recipe (`apps/website/src/content/docs/guides/providers.md:50-59`) documents a
provider block that can never admit a user.

Entra can attest verification through the optional `xms_edov` ("email domain owner
verified") claim, off by default and enabled per app registration by a tenant
administrator; tenants that cannot enable it have only the `email` claim, which Entra's
own guidance warns can be user-mutable in some tenant configurations. Both realities are
provider dialect, and provider dialect belongs at the adapter boundary — the Apple
adapter's bool-or-string coercion (`crates/adapters/src/shared/claims.rs:14`) set that
precedent. Weakening the core predicate, or trusting emails globally, would trade one
provider's gap for every provider's guarantee; a per-provider, default-off, explicit
override keeps the strict default while making a common upstream usable.

---

## Affected spec pages

| Canonical page | Nature of change |
|---|---|
| [`.specs/service/specs/05-provider-system.md`](../../service/specs/05-provider-system.md) | Add: an `## Email-verification overrides` section (modes, precedence, validation, startup warning, Entra recipe) and a pointer sentence in the Tier 1 paragraph; Modify: the `validate_id_token` behaviour bullet names the derivation; Add: three Decisions |
| [`.specs/service/specs/06-configuration.md`](../../service/specs/06-configuration.md) | Modify: the `[providers.<name>]` section documents the two oidc-adapter keys (prose only — the keys are validated in `provider_config_to_oidc`, like `endpoint_origins`, so no closed-domain table row) |
| [`.specs/service/specs/03-service-flows.md`](../../service/specs/03-service-flows.md) | Modify: step 4's two registration-policy bullets name the adapter-derived signal (the republished Found-active bullet also drops its allowlist-conditional framing, which the code has not matched since the unconditional predicate shipped — see the *Republished-text accuracy* Decision); Modify: the *Registration demands a verified email* Decision records that the overrides do not weaken the predicate |
| [`.specs/service/specs/01-domain-model.md`](../../service/specs/01-domain-model.md) | Modify: the `IdentityClaims` bullet describes `email_verified` as the adapter-derived verification signal; Modify: the `OidcProviderConfig` field enumeration gains `email_verification` (keeping the prose field list in step with the sidecar `$def`) |
| [`.specs/service/specs/canonical-types.schema.json`](../../service/specs/canonical-types.schema.json) | Add: `EmailVerification` `$def`; Modify: `OidcProviderConfig` gains the optional `email_verification` property |
| [`.specs/service/specs/02-ports-and-adapters.md`](../../service/specs/02-ports-and-adapters.md) | None — the `IdentityProvider` port signature is unchanged; derivation happens inside `validate_id_token`, behind the same contract |

---

## The delta

### Configuration surface

Two new optional keys in a `[providers.<name>]` block, meaningful only when
`adapter = "oidc"`. They are `extra`-map keys (`RawProviderConfig.extra`,
`crates/core/src/config.rs:1717-1723`), lifted and validated in the server's
`provider_config_to_oidc` (`crates/server/src/bootstrap.rs:1618-1714`) alongside
`client_id`, `scopes`, and `endpoint_origins`:

- `email_verified_claim` (string) — the name of an alternative claim to read,
  bool-or-string coerced through the shared `coerce_bool`, when the token's own
  `email_verified` claim is absent. Must be a non-empty string of at most 64 characters
  (the schema's `maxLength: 64`); a non-string TOML value, an empty string, or an
  oversized name is a `ConfigError` naming the provider.
- `trust_email_verified` (boolean, default `false`) — when `true`, a token whose
  `email_verified` claim is absent counts as verified iff it carries a non-empty `email`
  string claim. A set-but-non-boolean TOML value is a `ConfigError`, never coerced;
  an explicit `false` is identical to the key being absent.

Setting both keys on one provider block is a `ConfigError`. Neither key set is the
default and preserves today's behaviour exactly.

### Typed form

`EmailVerification` joins `OidcProviderConfig` in `crates/core/src/domain/provider.rs`:

```rust
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum EmailVerification {
    /// Read only the standard `email_verified` claim (current behaviour).
    #[default]
    Standard,
    /// An absent `email_verified` claim counts as verified iff the token
    /// carries a non-empty `email` string claim.
    TrustEmail,
    /// Read the named claim (bool-or-string coerced) when `email_verified`
    /// is absent, e.g. Entra's `xms_edov`.
    Claim(String),
}
```

`OidcProviderConfig` gains `pub email_verification: EmailVerification` (non-optional,
defaulted; it is configuration-grade — a host-name-class fact, not a credential — and
joins the hand-written `Debug` output at `provider.rs:37-59`). `provider_config_to_oidc`
constructs the variant from the two keys; the Apple adapter is untouched — Apple always
emits `email_verified` (bool or string) and its config parse reads its own keys.

### Derivation in the adapter

`OidcProvider` (`crates/adapters/src/oidc/mod.rs:34-42`) stores the mode from its config
(`from_config`, `mod.rs:114-123`), and `validate_id_token` (`mod.rs:187-201`) replaces
`email_verified: coerce_bool(&claims["email_verified"])` with the precedence rule:

1. `coerce_bool(claims["email_verified"])` — an explicit claim (`Some(true)` or
   `Some(false)`) always passes through. The overrides fill absence; they never overturn
   the provider's own signal, so `email_verified: false` is unverified in every mode.
2. When step 1 yields `None`: `Standard` → `None`; `Claim(name)` →
   `coerce_bool(claims[name])`; `TrustEmail` → `Some(true)` iff `claims["email"]` is a
   non-empty string, else `None`.

`IdentityClaims` is unchanged in shape — `email_verified: Option<bool>` — and the core
never reads raw claims for this decision; `registration_policy_reason` and its three call
sites are not touched by this change.

### Startup visibility

A provider whose resolved mode is not `Standard` logs exactly one structured warning at
registry build (`build_single_provider`, `crates/server/src/bootstrap.rs:1588-1608`),
naming the provider id and the mode (and the mapped claim name, for `Claim`). The toggle
weakens an identity control; it must be visible in boot logs the way the
endpoint-origin warning-mode logs and the role/listener collapse warnings are.

### Tests

- Adapter (wiremock, beside the existing `oidc/mod.rs` tests): under `Claim("xms_edov")`
  — boolean `true`, string `"true"`, absent (→ `None`), and explicit
  `email_verified: false` beside `xms_edov: true` (→ `Some(false)`); under `TrustEmail` —
  email present (→ `Some(true)`), email absent and empty-string email (→ `None`), and
  explicit `email_verified: false` (→ `Some(false)`); under `Standard` — absent stays
  `None` (pinning today's behaviour as a named case).
- `provider_config_to_oidc` (beside the test module at
  `crates/server/src/bootstrap.rs:1721`): each key lifted to its variant; both keys set →
  `ConfigError`; non-boolean `trust_email_verified`, non-string / empty / >64-character
  `email_verified_claim` → `ConfigError`; neither key → `Standard`.
- A resolve-level boot test (through `resolve_config_toml`) with an Entra-shaped block —
  `adapter = "oidc"`, a v2.0 issuer, `email_verified_claim = "xms_edov"` — asserting
  resolution succeeds.

---

## Proposed changes

### `.specs/service/specs/05-provider-system.md` → Tiers (Modify)

The Tier 1 paragraph's closing sentence — "Adding a Tier 1 provider is a config block —
no code." — is followed by:

> Two optional keys govern how the adapter derives `IdentityClaims.email_verified` for
> providers that do not emit the standard claim; see
> [Email-verification overrides](#email-verification-overrides).

### `.specs/service/specs/05-provider-system.md` → OidcProvider behaviour (Modify)

The `validate_id_token` bullet's closing sentence — "Both validators report it; neither
performs any replay or binding check itself." — is followed, inside the same bullet, by:

> The `email_verified` the returned claims carry is derived per the provider's configured
> email-verification mode (see
> [Email-verification overrides](#email-verification-overrides)); an explicit
> `email_verified` claim always passes through, bool-or-string coerced.

### `.specs/service/specs/05-provider-system.md` → new section (Add)

Between `` ## OidcProvider behaviour (`adapters/oidc`) `` and `## Provider registry`:

> ## Email-verification overrides
>
> The registration policy ([03-service-flows.md](03-service-flows.md)) accepts an email
> claim only when `IdentityClaims.email_verified == Some(true)`. The generic adapter
> derives that field per provider, in `validate_id_token`, from one of three configured
> modes:
>
> | Mode | Config | Derivation when the token's own `email_verified` is absent |
> |---|---|---|
> | Standard (default) | *(neither key set)* | stays `None` — a provider that does not attest verification cannot pass registration policy |
> | Mapped claim | `email_verified_claim = "<name>"` | read the named claim instead, bool-or-string coerced (`coerce_bool`); any other value is `None` |
> | Trusted email | `trust_email_verified = true` | `Some(true)` iff the token carries a non-empty `email` string claim |
>
> An explicit `email_verified` claim from the provider always passes through first — the
> overrides fill absence, they never overturn the provider's own signal, so a token
> carrying `email_verified: false` is unverified in every mode. Both keys are
> oidc-adapter `extra` keys, lifted and validated in the server's
> `provider_config_to_oidc` alongside `client_id` and `endpoint_origins`:
> `email_verified_claim` must be a non-empty string of at most 64 characters,
> `trust_email_verified` must be a TOML boolean (a set-but-non-boolean value is a config
> error, never coerced), and setting both on one provider block is a config error. The
> keys are meaningful only under `adapter = "oidc"`; the Apple adapter reads its own
> config and always receives `email_verified` from Apple. A provider configured with a
> non-default mode logs one structured startup warning at registry build naming the
> provider and the mode — the toggle weakens an identity control and must be visible in
> boot logs.
>
> **Microsoft Entra ID (Azure AD) v2.0** is the motivating deployment: its id_tokens
> carry `email` but no `email_verified` claim (scopes `openid email profile`), so under
> the default mode every Entra sign-in fails registration policy. The supported recipe
> maps Entra's optional `xms_edov` ("email domain owner verified") claim, which a tenant
> administrator enables per app registration:
>
> ```toml
> [providers.entra]
> adapter = "oidc"
> issuer = "https://login.microsoftonline.com/${ENTRA_TENANT_ID}/v2.0"
> client_id = "${ENTRA_CLIENT_ID}"
> client_secret = "${ENTRA_CLIENT_SECRET}"
> scopes = ["openid", "email", "profile"]
> email_verified_claim = "xms_edov"
> ```
>
> `xms_edov` is off by default in Entra; tenants that cannot enable it may instead set
> `trust_email_verified = true`, accepting that Entra's `email` claim is then trusted
> without domain-ownership proof — Entra documents the claim as user-mutable in some
> tenant configurations, which is exactly the input class `xms_edov` exists to attest.
> Prefer the claim mapping wherever the tenant permits it.

### `.specs/service/specs/05-provider-system.md` → Assumptions and open questions → Decisions (Add)

> - *Verification is derived at the adapter boundary; the policy predicate is untouched.*
>   **`registration_policy_reason` still requires `email_verified == Some(true)` on every
>   path; the per-provider modes change only how the generic adapter derives that
>   field.** The core cannot know which claim attests verification for which provider —
>   that is provider dialect, and provider dialect lives in adapters (the Apple
>   bool-or-string coercion set the precedent). Keeping the predicate closed also keeps
>   the security-review surface one function.
> - *Overrides fill absence, never overturn.* **An explicit `email_verified` claim wins
>   over any configured override, in both directions.** A provider that says `false` has
>   made a statement; a configuration that discarded it would turn a per-provider
>   gap-filler into a general verification bypass.
> - *Explicit, per-provider, default-strict.* **Both keys default off, are scoped to one
>   provider block, and are mutually exclusive.** Trusting an unverified email is a
>   security-sensitive weakening — a domain-allowlist entry could be satisfied by an
>   attacker-authored address, and downstream consumers of `user.email` (user-sync
>   webhooks, admin surfaces) inherit the trust — so it must be a deliberate, auditable,
>   per-provider choice, never a global flag and never a side effect of a provider's
>   claim shape.

### `.specs/service/specs/06-configuration.md` → `[providers.<name>]` (Modify)

Two edits to the section's single paragraph. First, its final sentence — the bare "See
[05-provider-system.md](05-provider-system.md)." — is deleted (the anchored link that
closes the new paragraph subsumes it). Second, this new paragraph is appended after the
now-shortened existing paragraph, as the section's second paragraph:

> Two optional oidc-adapter keys govern how the adapter derives
> `IdentityClaims.email_verified` for providers that do not emit the standard
> `email_verified` claim (Microsoft Entra ID v2.0 is the motivating case):
> `email_verified_claim` (non-empty string, at most 64 characters — read the named claim,
> bool-or-string coerced, when the standard claim is absent) and `trust_email_verified`
> (TOML boolean, default `false` — treat a non-empty `email` claim as verified when the
> standard claim is absent). An explicit `email_verified` claim from the provider always
> takes precedence, setting both keys on one provider block is a config error, and both
> are validated in the same `provider_config_to_oidc` lift as the other adapter-specific
> fields — a set-but-mistyped value fails registry build rather than being coerced or
> ignored. See
> [05-provider-system.md](05-provider-system.md#email-verification-overrides).

### `.specs/service/specs/03-service-flows.md` → Token exchange → step 4 (Modify)

The **Found, active** bullet currently conditions the re-check on
`registration.domain_allowlist` being set — framing the shipped unconditional predicate
has never matched. It becomes:

> - **Found, active** → re-apply the registration policy against the assertion's current
>   claims, through the same predicate as the Not-found arm: a verified email
>   (`email_verified == Some(true)`) is always required, and when
>   `registration.domain_allowlist` is set the email's domain must also match it. A
>   failure → `AccessDenied` (audited `RegistrationDenied`, naming the user id). The live
>   ID-token claims are used rather than the stored `user.email`, which is frozen at
>   first login. `registration.mode` is not re-evaluated here: it is an admission gate
>   and is trivially satisfied by an existing user.

The Not-found arm's verified-email bullet becomes:

> - The ID token must carry a **verified** email (`email_verified == Some(true)`) — a
>   requirement of accepting the claim at all, not merely of the allowlist branch. A
>   missing or unverified email → `AccessDenied` (audited `RegistrationDenied`).
>   `email_verified` is the signal the provider adapter derived — for the generic OIDC
>   adapter, per that provider's configured email-verification mode
>   ([05-provider-system.md](05-provider-system.md#email-verification-overrides)); the
>   policy itself never reads raw claims.

(The suspended arm, the allowlist bullet, the `RegistrationMode` bullets, and the rest of
the step are unchanged.)

### `.specs/service/specs/03-service-flows.md` → Assumptions and open questions (Modify)

The Decision *Registration demands a verified email.* becomes:

> - *Registration demands a verified email.* **Every just-in-time user creation requires
>   `email_verified == true`, whether or not an allowlist is configured.** The
>   requirement is a property of accepting the email claim, not of the allowlist; nesting
>   it inside an optional feature's branch meant turning the allowlist off turned
>   identity verification off with it. The per-provider email-verification overrides
>   ([05-provider-system.md](05-provider-system.md#email-verification-overrides)) do not
>   weaken this predicate: they govern how the generic OIDC adapter derives
>   `email_verified` from a provider's claims, and the core still refuses anything short
>   of `Some(true)`.

(The surrounding Decisions are unchanged.)

### `.specs/service/specs/01-domain-model.md` → Token types (Modify)

The `IdentityClaims` bullet becomes:

> - **`IdentityClaims`** — verified claims from a provider ID token: `subject`, optional
>   `email`, `email_verified`, `name`, `is_private_email` (Apple private-relay flag;
>   `None` for other providers), `signing_alg` (the algorithm the resolved JWK verified
>   with, e.g. `"ES256"`), and `raw_claims`. `email_verified` is the adapter-derived
>   verification signal the registration policy reads: an explicit `email_verified`
>   claim passes through (bool-or-string coerced), and for the generic OIDC adapter an
>   absent claim may be filled by that provider's configured email-verification override
>   ([05-provider-system.md](05-provider-system.md#email-verification-overrides)).

### `.specs/service/specs/01-domain-model.md` → OidcProviderConfig (Modify)

The single paragraph under `` ### OidcProviderConfig (`domain/provider.rs`) `` becomes
(the field enumeration gains `email_verification`; everything before `scopes` is
unchanged):

> The normalized config the standard OIDC adapter consumes: `provider_id`, `issuer`,
> `client_id`, optional `client_secret` (a `Secret<String>` — unprintable by type), optional
> `jwks_uri` / `token_endpoint` / `revocation_endpoint` (discovered from the issuer if
> absent), optional `endpoint_origins` (extra origins a discovery document may name; see
> [05-provider-system.md](05-provider-system.md)), `scopes`, `additional_params`, and
> `email_verification` — the per-provider email-verification derivation mode, default
> `Standard` ([05-provider-system.md](05-provider-system.md#email-verification-overrides)).

---

## Type changes

Fragment for `.specs/service/specs/canonical-types.schema.json`. `EmailVerification` is a
new `$def`; `OidcProviderConfig` is a modified entity shown complete in its post-merge
shape (replace the sidecar's `$def` wholesale) — its `$comment` enumerates the diff.

```json
{
  "$comment": "Fragment for 2026-08-31-per_provider_email_verification_overrides. EmailVerification is a new $def; OidcProviderConfig is modified — replace its $def wholesale.",
  "$defs": {
    "EmailVerification": {
      "description": "How the generic OIDC adapter derives IdentityClaims.email_verified when the token's own email_verified claim is absent; an explicit claim always passes through and can never be overturned. standard: absent stays null. trust_email: a non-empty email claim counts as verified. claim: read the named claim, bool-or-string coerced.",
      "default": "standard",
      "oneOf": [
        { "const": "standard" },
        { "const": "trust_email" },
        {
          "type": "object",
          "required": ["claim"],
          "additionalProperties": false,
          "properties": {
            "claim": {
              "$ref": "../../canonical-types.schema.json#/$defs/NonEmptyString",
              "maxLength": 64,
              "description": "Claim name to read, e.g. Entra's xms_edov."
            }
          }
        }
      ]
    },
    "OidcProviderConfig": {
      "$comment": "Complete post-merge shape; diff vs the current sidecar: the optional email_verification property (default standard) is added. All other properties, required, and descriptions are retained unchanged.",
      "type": "object",
      "required": [
        "provider_id",
        "issuer",
        "client_id",
        "scopes"
      ],
      "properties": {
        "provider_id": {
          "type": "string"
        },
        "issuer": {
          "type": "string",
          "description": "Used for OIDC discovery. Its origin is always in the permitted endpoint-origin set."
        },
        "client_id": {
          "type": "string"
        },
        "client_secret": {
          "type": [
            "string",
            "null"
          ]
        },
        "jwks_uri": {
          "type": [
            "string",
            "null"
          ]
        },
        "token_endpoint": {
          "type": [
            "string",
            "null"
          ]
        },
        "revocation_endpoint": {
          "type": [
            "string",
            "null"
          ]
        },
        "endpoint_origins": {
          "type": "array",
          "default": [],
          "items": {
            "type": "string",
            "pattern": "^https://[^/?#]+$"
          },
          "description": "Extra origins a discovery document may name for token_endpoint, jwks_uri, or revocation_endpoint, beyond the issuer's own origin and those of explicitly configured endpoints. Scheme, host, optional port; no path, query, or fragment."
        },
        "scopes": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "additional_params": {
          "type": "object",
          "additionalProperties": {
            "type": "string"
          }
        },
        "email_verification": {
          "$ref": "#/$defs/EmailVerification",
          "default": "standard",
          "description": "Per-provider email-verification derivation mode; configured via the email_verified_claim / trust_email_verified provider keys."
        }
      }
    }
  }
}
```

`IdentityClaims` is unchanged in shape and is not republished.

---

## Implementation notes

Suggested order — the typed form first, then the lift, then the adapter, then tests and
docs:

```
1. core   crates/core/src/domain/provider.rs:8-59 — EmailVerification enum (Default =
          Standard) + email_verification field on OidcProviderConfig, joining the
          hand-written Debug; re-export beside OidcProviderConfig
          (crates/core/src/domain/mod.rs:18). Update the struct's constructors:
          provider.rs sample_config (tests), adapters oidc make_config
          (crates/adapters/src/oidc/mod.rs:338-356), bootstrap tests.
2. server crates/server/src/bootstrap.rs:1618-1714 (provider_config_to_oidc) — lift
          trust_email_verified via toml::Value::as_bool and email_verified_claim via
          as_str; ConfigError on: both set, set-but-non-boolean, non-string / empty /
          >64-character claim name. Startup warning for a non-Standard mode beside
          build_single_provider (bootstrap.rs:1588-1608).
3. adapter crates/adapters/src/oidc/mod.rs:34-42 (field), 114-123 (from_config), 187-201
          (derivation per the precedence rule, reusing coerce_bool from
          crates/adapters/src/shared/claims.rs:14). crates/providers/src/apple.rs is
          untouched.
4. tests  adapter wiremock cases per mode + precedence (beside the existing oidc tests);
          provider_config_to_oidc validation cases (beside bootstrap.rs:1721); an
          Entra-shaped resolve_config_toml boot test. Core registration-policy tests
          (crates/core/tests/exchange.rs:929-1013) stand unchanged — the core is not
          touched.
5. docs   apps/website/src/content/docs/guides/providers.md — add the two keys to the
          field table (:36-46) and extend the Microsoft Entra ID example (:50-59) with
          email_verified_claim = "xms_edov", the xms_edov enablement note, and the
          trust_email_verified fallback. Content-only; the website spec
          (.specs/website/specs/00-overview.md) inventories the docs tree at section
          level (getting-started, guides, deployment, architecture, contributing), not
          per page, so a content edit inside guides/providers.md needs no website spec
          page changes.
```

References: issue #48 (observed Entra v2.0 claim set; 0.4.0); Microsoft Entra optional
claims documentation for `xms_edov` and the mutability caveat on `email`; the Apple
coercion change
([`2026-07-01-require_iss_aud_in_token_validation.md`](2026-07-01-require_iss_aud_in_token_validation.md))
as the precedent for provider-dialect handling at the adapter boundary.

---

## Compatibility and migration

- **Strictly additive.** With neither key set, claim derivation, registration policy,
  wire shapes, stored sessions, and audit events are byte-identical to 0.4.0. No
  persistence, schema, FFI, or binding surface changes; config flows through the shared
  resolve as TOML on every entry point, and unknown `extra` keys were already
  representable.
- Existing configs that happen to carry either key (previously inert, silently ignored)
  now take effect — or fail registry build if mistyped or contradictory. Fail-closed and
  deliberate: an operator who wrote `trust_email_verified = true` against 0.4.0 believed
  it worked; after this change it does, visibly, with a startup warning.
- Deployments enabling an override should re-check their `registration.domain_allowlist`
  intent first: under `trust_email_verified = true` an allowlist match no longer implies
  the account holder controls a mailbox in that domain.

---

## Merge plan

1. Apply the nine `Proposed changes` blocks to `05-provider-system.md`,
   `06-configuration.md`, `03-service-flows.md`, and `01-domain-model.md`; bump each
   page's `**Date:**` to the merge date.
2. Fold the `Type changes` fragment into
   `.specs/service/specs/canonical-types.schema.json`: add the `EmailVerification`
   `$def`, replace the `OidcProviderConfig` `$def` wholesale with the fragment's. Drop
   the change-tracking `$comment`s on the way in.
3. Verify the merged 05 section against the shipped code: the precedence rule, the
   validation errors, the startup warning, and that the Entra recipe boots through
   `resolve_config_toml`.
4. Flip this file's `**Status:**` to `Merged`, add `**Merged:** YYYY-MM-DD`, move it to
   `.specs/changes/merged/`.
5. Update `.specs/README.md`: move this spec's row from pending to merged.

---

## Assumptions and open questions

### Assumptions

- Issue #48's empirical claim set holds: an Entra v2.0 id_token for `openid email
  profile` carries `email` and no `email_verified`; `xms_edov` is optional, off by
  default, and tenant-enabled per app registration. Entra may emit `xms_edov` as a JSON
  boolean or a string — either coerces through the shared `coerce_bool`, and any other
  value (e.g. a number) yields `None` and a denial.
- `IdentityClaims.email_verified` is read only by `registration_policy_reason`'s three
  call sites in `exchange.rs`; no other flow, binding control, or persistence path
  consumes it, so moving its derivation into the adapter changes exactly the
  registration decision.
- The FFI and Lambda entry points reach providers through the same
  `bootstrap::build_single_provider` path as the server, so the lift, validation, and
  startup warning apply on every runtime.

### Decisions

- *Adapter-boundary derivation, not a core policy flag.* **The override changes how
  `OidcProvider` fills `IdentityClaims.email_verified`; `registration_policy_reason` and
  its call sites are untouched.** A per-provider policy flag in the core would need
  provider config threaded into the exchange flow and would open the closed predicate;
  the adapter already owns provider dialect (Apple's coercion), already returns the
  field, and keeps the security review surface one function.
- *Two flat keys, not one polymorphic key.* **`email_verified_claim` (string) and
  `trust_email_verified` (bool), mutually exclusive, over a single key with keyword and
  claim-name values mixed.** A single string key would make a claim literally named
  `trust` unrepresentable and would hide the security-relevant difference between "map a
  provider attestation" and "waive attestation"; two keys keep each mode independently
  greppable, and the exclusivity check is one config error.
- *Explicit claim always wins.* **Step 1 of the precedence rule passes an explicit
  `email_verified` through in both directions before any override applies.** An override
  that could overturn `false` would be a verification bypass; one that could overturn
  `true` would be pointless. Filling absence only is the narrowest semantics that fixes
  Entra.
- *Trust mode still requires an email.* **`TrustEmail` yields `Some(true)` only for a
  non-empty `email` string claim; otherwise `None`.** The mode asserts "this provider's
  email is verified", not "this provider needs no email" — a subject without an email
  still fails the policy's first check, unchanged.
- *Validation lives in `provider_config_to_oidc`.* **The keys are validated at the same
  boundary as `client_id`, `scopes`, and `endpoint_origins` — registry build — not in
  `Config::resolve`.** That is the documented home of the oidc adapter's typed lift
  (`06-configuration.md`), and splitting one adapter's keys across two validation
  boundaries would be a second asymmetry. Known consequence, accepted for consistency: a
  `role = "admin"` deployment builds no registry, so a mistyped override — like a missing
  `client_id` today — surfaces only on roles that serve `/token`.
- *A 64-character claim-name bound.* **`email_verified_claim` is capped at 64 characters
  — the schema fragment's `maxLength: 64`, counted in Unicode code points — and must be
  non-empty.** Real claim names (`xms_edov`, `email_verified`, even vendor-namespaced
  names) stay well under it; the bound keeps hostile config text out of error messages
  and log lines, matching the endpoint-origins length discipline.
- *Startup warning, no per-event audit annotation.* **Visibility is one structured
  warning per configured provider at registry build; audit events are unchanged.** The
  mode is static configuration, identical for every event a provider produces —
  per-event annotation would say the same thing millions of times, and the event wire
  shape stays stable for SIEM consumers. The boot log names the weakening once, where
  deployment reviews look for it.
- *Scoped to the generic adapter.* **The keys mean nothing under `adapter = "apple"` and,
  like every other adapter-specific `extra` key on a foreign adapter, are inert there.**
  Apple always attests `email_verified`; inventing cross-adapter key validation is a
  separate (pre-existing) question this change does not open.
- *Republished-text accuracy.* **The republished Found-active bullet in
  `03-service-flows.md` also drops its "when `registration.domain_allowlist` is set"
  framing.** The shipped predicate is unconditional for existing users
  (`exchange.rs:331-342`; the R2 review's S14 confirmed the code is intended), and a
  sentence this change republishes must be true on merge — the same rule the R2 change
  spec applied. Nothing else of that deferred doc-only pass folds in here.
- *The website recipe is part of the fix.* **The Entra example in the providers guide is
  updated in the same change, not left for a docs pass.** The guide currently documents a
  provider block that cannot admit a user; shipping the capability while the recipe still
  omits the override key would reproduce issue #48 for every reader.

### Open questions

(None at this stage.)
