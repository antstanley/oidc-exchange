# Change: Require iss/aud presence and fix claim handling in ID-token validation

**Status:** Proposed · **Date:** 2026-07-01 · **Owner:** Ant Stanley · **Target:** crates/adapters (oidc), crates/providers (apple)

Make ID-token validation in both identity providers reject tokens that _omit_ the `iss` or
`aud` claim, validate `nbf`, infer the signing algorithm from the JWK's `kty`/`crv` when the
JWK carries no `alg`, accept Apple's string-typed `email_verified`, and surface Apple's
`is_private_email` as a first-class `IdentityClaims` field. Today a
provider-signed JWT with a valid `kid`, `exp`, and `sub` but no `aud`/`iss` claim passes
validation, and every Apple sign-in is denied when a registration domain allowlist is on.

---

## Motivation

Both providers build `Validation::new(alg)` and call `set_issuer`/`set_audience`
(`crates/adapters/src/oidc/mod.rs:137-139`, `crates/providers/src/apple.rs:260-262`). In
jsonwebtoken 10.x only `exp` is in `required_spec_claims`; the iss/aud match-arms fall
through when the claim is absent, so a token _without_ those claims validates. Any JWT
signed by the provider's keys — e.g. a Keycloak realm access token, which omits `aud` — is
accepted as an ID token (cross-token-type confusion). Separately,
`crates/adapters/src/oidc/mod.rs:136` silently defaults a JWK with no `alg` to RS256,
diverging from the [05-provider-system](../service/specs/05-provider-system.md)
"algorithm from the JWK" decision and breaking Azure-AD-style JWKS with non-RS256 keys.

On the claim-mapping side, `crates/providers/src/apple.rs:283` reads
`claims["email_verified"].as_bool()`, but Apple frequently sends `"email_verified": "true"`
as a JSON string, yielding `None`. Core's registration allowlist requires
`email_verified == Some(true)` (`crates/core/src/service/exchange.rs:104`), so every Apple
sign-in is denied under an allowlist. Finally, `nbf` is never validated
(`validate_nbf` is false by default in both providers).

---

## Affected spec pages

| Canonical page                                                                         | Nature of change                                                                                                                                                                                                                                                                                                                             |
| -------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [`.specs/service/specs/05-provider-system.md`](../service/specs/05-provider-system.md) | Tighten the `OidcProvider behaviour` validation description (required claims, `nbf`, alg inference); add Apple claim-coercion note, including the surfaced `is_private_email` field; add a required-claims Decision. The existing _Algorithm from the JWK_ Decision already states the desired end-state — the code, not the spec, diverges — so the delta only sharpens the behaviour prose. |

[02-ports-and-adapters.md](../service/specs/02-ports-and-adapters.md) is unaffected: no
port signature changes. The outbound-HTTP fixes to the same code paths are split into
[2026-07-01-harden_outbound_provider_http.md](2026-07-01-harden_outbound_provider_http.md).

---

## Proposed changes

### `.specs/service/specs/05-provider-system.md` → OidcProvider behaviour (Modify)

> - `validate_id_token` decodes the JWT header, fetches the issuer's JWKS through the cached
>   `JwksCache`, and validates the signature using the **algorithm from the JWK** (not the
>   untrusted header). Validation requires the `exp`, `iss`, and `aud` claims to be
>   **present** (`set_required_spec_claims`) and to match the configured issuer and
>   `client_id`; `nbf` is validated when present. A token missing `iss` or `aud` — e.g. a
>   provider access token presented as an ID token — is rejected.
> - When the matched JWK carries no `alg`, the algorithm is inferred from the key type:
>   `kty: EC` by `crv` (P-256 → ES256, P-384 → ES384), `kty: OKP` → EdDSA, `kty: RSA` →
>   RS256. Any other alg-less key is rejected. (Azure-AD-style JWKS omit `alg`.)

### `.specs/service/specs/05-provider-system.md` → Tiers, Tier 2 Apple (Modify)

> Apple's ID tokens sometimes carry `email_verified` (and `is_private_email`) as the JSON
> strings `"true"`/`"false"` rather than booleans. The Apple provider coerces bool-or-string
> values when mapping to `IdentityClaims.email_verified` and
> `IdentityClaims.is_private_email`, so the registration domain allowlist (which requires
> `email_verified == Some(true)`) works for Apple sign-ins. `is_private_email` is a
> first-class `Option<bool>` field on `IdentityClaims`, populated only by the Apple
> provider; the generic OIDC provider leaves it `None`.

### `.specs/service/specs/05-provider-system.md` → Decisions (Add)

> - _Required spec claims._ **ID-token validation requires `exp`, `iss`, and `aud` to be
>   present, not merely correct-when-present.** Closes the cross-token-type confusion class
>   (e.g. Keycloak realm access tokens omit `aud`).

---

## Type changes

`IdentityClaims` gains an optional `is_private_email` field (Apple-only; the generic OIDC
provider leaves it null). Folds into the existing `IdentityClaims` definition in
[`canonical-types.schema.json`](../service/specs/canonical-types.schema.json).

```json
{
  "$comment": "Fragment for 2026-07-01-require_iss_aud_in_token_validation. Folds into .specs/service/specs/canonical-types.schema.json#/$defs/IdentityClaims on merge.",
  "$defs": {
    "IdentityClaims": {
      "properties": {
        "is_private_email": {
          "type": ["boolean", "null"],
          "description": "Apple private-relay flag, coerced bool-or-string like email_verified; null for non-Apple providers."
        }
      }
    }
  }
}
```

---

## Implementation notes

1. `crates/adapters/src/oidc/mod.rs:137-139` — after `Validation::new(jwk_alg)`, add
   `validation.set_required_spec_claims(&["exp", "iss", "aud"])` and
   `validation.validate_nbf = true`.
2. `crates/providers/src/apple.rs:260-262` — same two lines.
3. `crates/adapters/src/oidc/mod.rs:121-136` — replace `.unwrap_or(Algorithm::RS256)` with
   inference from the JWK's `kty`/`crv` when `alg` is absent; error on anything
   unrecognised (mirror the Apple provider's error at `crates/providers/src/apple.rs:249-259`).
4. `crates/providers/src/apple.rs:283` — coerce bool-or-string (`"true"`/`"false"`) for
   `email_verified`; apply the same helper at `crates/adapters/src/oidc/mod.rs:160` for
   consistency. Put the helper in `crates/adapters/src/shared` since both crates use it.
5. `crates/core/src/domain/token.rs:74-81` — add `is_private_email: Option<bool>` to
   `IdentityClaims` (the struct lives in `domain`; the port at
   `crates/core/src/ports/identity_provider.rs:12` returns it unchanged). Populate it in the
   Apple provider's claim mapping at `crates/providers/src/apple.rs:280-289` using the same
   bool-or-string coercion helper; the generic OIDC provider leaves it `None`.
6. Tests: token with no `aud`/`iss` rejected, `nbf` in the future rejected, alg-less RSA and
   EC JWKs validated, string `email_verified` mapped to `Some(true)` — in both providers'
   test modules (`oidc/mod.rs` tests use `generate_rsa_test_keys`, `apple.rs` tests use
   `generate_es256_test_keys`). In `apple.rs`, string and bool `is_private_email` both map
   to `Some(_)`.

---

## Merge plan

1. Apply the three `Proposed changes` blocks to
   [05-provider-system.md](../service/specs/05-provider-system.md); bump its `**Date:**`.
2. Fold the `Type changes` fragment into
   [`canonical-types.schema.json`](../service/specs/canonical-types.schema.json)
   (`$defs/IdentityClaims`).
3. Flip `**Status:**` to `Merged`, stamp `**Merged:**`, move to `.specs/changes/merged/`.
4. Update `.specs/README.md`.

---

## Assumptions and open questions

### Assumptions

- No supported provider issues legitimate ID tokens without `iss`/`aud`; requiring presence
  breaks no working configuration (OIDC Core mandates both claims in ID tokens).
- `nbf` is rarely present in ID tokens; enabling `validate_nbf` (with jsonwebtoken's default
  leeway) does not reject valid tokens.

### Decisions

- _RSA without `alg` means RS256._ **An alg-less `kty: RSA` JWK is treated as RS256.** The
  RSA family (RS/PS, 256/384/512) is not distinguishable from key parameters alone, and the
  untrusted token header must not decide; RS256 matches Azure AD's actual signing algorithm.
- _Coercion is shared, not Apple-only._ **The bool-or-string coercion lives in
  `adapters/shared` and both providers use it.** Keycloak/Google send booleans today, but the
  coercion is harmless and keeps the two `validate_id_token` bodies aligned.
- _Surface `is_private_email`._ **`IdentityClaims` gains `is_private_email: Option<bool>`,
  populated by the Apple provider with the same bool-or-string coercion; the generic OIDC
  provider leaves it `None`.** Downstream consumers should not have to dig through
  `raw_claims` for a claim the coercion already normalises.

### Open questions

- (None at this stage.)
