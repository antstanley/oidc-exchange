# Task 03 — harden OIDC validate_id_token (required claims, nbf, alg inference, coercion)

**Plan:** [plan.md](../plan.md) · **Certificate:** [03-oidc_validate_id_token_hardening-certificate.md](03-oidc_validate_id_token_hardening-certificate.md)

**Implements:** [05-provider-system.md](../../../service/specs/05-provider-system.md) §"OidcProvider behaviour" (required `exp`/`iss`/`aud` presence, `nbf`-when-present, alg inference from the JWK `kty`/`crv`) and §Decisions *Required spec claims* and *RSA without `alg` means RS256*.
**Depends on:** 01, 02
**Produces:** the generic OIDC adapter rejects an ID token that omits `iss` or `aud`, validates `nbf` when present, infers the signing algorithm from the JWK's `kty`/`crv` when the JWK carries no `alg` (RSA→RS256, EC P-256→ES256 / P-384→ES384, OKP→EdDSA, else reject), and coerces `email_verified` via the shared helper.
**Pointers:** `crates/adapters/src/oidc/mod.rs:120-139` (alg selection + `Validation` build), `:136` (the `.unwrap_or(Algorithm::RS256)` to replace), `:160` (the `email_verified` mapping); mirror the alg-reject pattern from `crates/providers/src/apple.rs:249-259`; shared helper from task 01 (`oidc_exchange_adapters::shared::claims::coerce_bool`); tests use `generate_rsa_test_keys` in the `oidc/mod.rs` test module.

## Steps

- [ ] After `let mut validation = Validation::new(jwk_alg);`, add `validation.set_required_spec_claims(&["exp", "iss", "aud"])` and `validation.validate_nbf = true` (before or alongside `set_issuer`/`set_audience`).
- [ ] Replace the `.unwrap_or(Algorithm::RS256)` fallback at `oidc/mod.rs:136` with inference when `alg` is absent: read the JWK's `kty` (and `crv` for EC) — `RSA`→`RS256`, `EC` `P-256`→`ES256` / `P-384`→`ES384`, `OKP`→`EdDSA`; any other alg-less key returns `Error::InvalidGrant` (mirror `apple.rs:249-259`). A JWK that does carry a recognised `alg` string still uses it.
- [ ] Replace `claims["email_verified"].as_bool()` at `oidc/mod.rs:160` with `coerce_bool(&claims["email_verified"])`; leave `is_private_email: None` in this constructor (generic OIDC does not surface it).
- [ ] Add tests in the `oidc/mod.rs` module: token missing `aud` rejected, token missing `iss` rejected, future-`nbf` token rejected, alg-less RSA JWK validated, alg-less EC (P-256) JWK validated, and a string `email_verified: "true"` mapped to `Some(true)`.
- [ ] Ensure the touched functions carry at least two meaningful assertions and any new bound is a named constant.

## Definition of done

- [ ] An ID token omitting `iss` or `aud`, and a token whose `nbf` is in the future, are each rejected with `Error::InvalidGrant`; a well-formed token still validates.
- [ ] An alg-less RSA JWK validates as RS256 and an alg-less EC P-256 JWK validates as ES256; an alg-less key of an unrecognised type is rejected.
- [ ] Negative-space tests cover each new rejection path (missing `aud`, missing `iss`, future `nbf`, unrecognised alg-less key), and a string `email_verified` maps to `Some(true)`.
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: a reviewer runs the `oidc/mod.rs` tests and sees the missing-`iss`/`aud`, future-`nbf`, alg-less RSA/EC, and string-`email_verified` cases all behave as specified.
