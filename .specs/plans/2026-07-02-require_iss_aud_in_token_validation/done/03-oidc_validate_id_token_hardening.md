# Task 03 — harden OIDC validate_id_token (required claims, nbf, alg inference, coercion)

**Plan:** [plan.md](../plan.md) · **Certificate:** [03-oidc_validate_id_token_hardening-certificate.md](03-oidc_validate_id_token_hardening-certificate.md)

**Implements:** [05-provider-system.md](../../../service/specs/05-provider-system.md) §"OidcProvider behaviour" (required `exp`/`iss`/`aud` presence, `nbf`-when-present, alg inference from the JWK `kty`/`crv`) and §Decisions *Required spec claims* and *RSA without `alg` means RS256*.
**Depends on:** 01, 02
**Produces:** the generic OIDC adapter rejects an ID token that omits `iss` or `aud`, validates `nbf` when present, infers the signing algorithm from the JWK's `kty`/`crv` when the JWK carries no `alg` (RSA→RS256, EC P-256→ES256 / P-384→ES384, OKP→EdDSA, else reject), and coerces `email_verified` via the shared helper; and `05-provider-system.md` §"OidcProvider behaviour" + §Decisions *Required spec claims* are updated to the merged form (page `**Date:**` bumped).
**Pointers:** `crates/adapters/src/oidc/mod.rs:120-139` (alg selection + `Validation` build), `:136` (the `.unwrap_or(Algorithm::RS256)` to replace), `:160` (the `email_verified` mapping); mirror the alg-reject pattern from `crates/providers/src/apple.rs:249-259`; shared helper from task 01 (`oidc_exchange_adapters::shared::claims::coerce_bool`); tests use `generate_rsa_test_keys` in the `oidc/mod.rs` test module.

## Steps

- [x] After `let mut validation = Validation::new(jwk_alg);`, add `validation.set_required_spec_claims(&["exp", "iss", "aud"])` and `validation.validate_nbf = true` (before or alongside `set_issuer`/`set_audience`).
- [x] Replace the `.unwrap_or(Algorithm::RS256)` fallback at `oidc/mod.rs:136` with inference when `alg` is absent: read the JWK's `kty` (and `crv` for EC) — `RSA`→`RS256`, `EC` `P-256`→`ES256` / `P-384`→`ES384`, `OKP`→`EdDSA`; any other alg-less key returns `Error::InvalidGrant` (mirror `apple.rs:249-259`). A JWK that does carry a recognised `alg` string still uses it.
- [x] Replace `claims["email_verified"].as_bool()` at `oidc/mod.rs:160` with `coerce_bool(&claims["email_verified"])`; leave `is_private_email: None` in this constructor (generic OIDC does not surface it).
- [x] Add tests in the `oidc/mod.rs` module: token missing `aud` rejected, token missing `iss` rejected, future-`nbf` token rejected, alg-less RSA JWK validated, alg-less EC (P-256) JWK validated, and a string `email_verified: "true"` mapped to `Some(true)`.
- [x] Apply the change spec's two `.specs/service/specs/05-provider-system.md` blocks this task realises: under §"OidcProvider behaviour", replace the single `validate_id_token` bullet with the two-bullet merged form (required `exp`/`iss`/`aud` presence via `set_required_spec_claims` and match, `nbf`-when-present, and alg inference from the JWK's `kty`/`crv` with the alg-less-key reject); and add the *Required spec claims* Decision under §Decisions. Bump the page's `**Date:**` to `2026-07-02`. (Task 04 applies the §"Tiers, Tier 2 Apple" block to a different section of the same page and sets the same `**Date:**` value — the two edits merge cleanly.)
- [x] Ensure the touched functions carry at least two meaningful assertions and any new bound is a named constant.

## Definition of done

- [x] An ID token omitting `iss` or `aud`, and a token whose `nbf` is in the future, are each rejected with `Error::InvalidGrant`; a well-formed token still validates.
- [x] An alg-less RSA JWK validates as RS256 and an alg-less EC P-256 JWK validates as ES256; an alg-less key of an unrecognised type is rejected.
- [x] Negative-space tests cover each new rejection path (missing `aud`, missing `iss`, future `nbf`, unrecognised alg-less key), and a string `email_verified` maps to `Some(true)`.
- [x] `.specs/service/specs/05-provider-system.md` §"OidcProvider behaviour" describes the required `exp`/`iss`/`aud` presence, `nbf`-when-present, and JWK `kty`/`crv` alg inference in its merged form, §Decisions carries the *Required spec claims* entry, and the page `**Date:**` is bumped — moved together with this code change.
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: a reviewer runs the `oidc/mod.rs` tests and sees the missing-`iss`/`aud`, future-`nbf`, alg-less RSA/EC, and string-`email_verified` cases all behave as specified, and confirms 05-provider-system.md §"OidcProvider behaviour" + §Decisions *Required spec claims* now match the merged form.
