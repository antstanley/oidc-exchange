# Task 04 — harden Apple validate_id_token (required claims, nbf, coercion, is_private_email)

**Plan:** [plan.md](../plan.md) · **Certificate:** [04-apple_validate_id_token_hardening-certificate.md](04-apple_validate_id_token_hardening-certificate.md)

**Implements:** [05-provider-system.md](../../../service/specs/05-provider-system.md) §"OidcProvider behaviour" (required `exp`/`iss`/`aud` presence, `nbf`-when-present) and §"Tiers, Tier 2 Apple" (bool-or-string coercion of `email_verified` and `is_private_email`, the latter surfaced as a first-class field); realises the change spec's fix for Apple sign-ins being denied under a registration domain allowlist.
**Depends on:** 01, 02
**Produces:** the Apple provider rejects an ID token that omits `iss` or `aud`, validates `nbf` when present, coerces `email_verified` via the shared helper (so `"true"` → `Some(true)`), and populates `is_private_email` from the same bool-or-string coercion; and `05-provider-system.md` §"Tiers, Tier 2 Apple" is updated with the Apple coercion note (page `**Date:**` bumped).
**Pointers:** `crates/providers/src/apple.rs:260-262` (`Validation` build — add the two lines), `:283` (`email_verified` mapping), `:280-289` (the `IdentityClaims` constructor — set `is_private_email`); shared helper from task 01 (`oidc_exchange_adapters::shared::claims::coerce_bool`); tests use `generate_es256_test_keys` in the `apple.rs` test module. The Apple alg path (`:249-259`) already errors on missing/unrecognised `alg` — no alg-inference change here.

## Steps

- [ ] After `let mut validation = Validation::new(jwk_alg);` in `apple.rs`, add `validation.set_required_spec_claims(&["exp", "iss", "aud"])` and `validation.validate_nbf = true`.
- [ ] Replace `claims["email_verified"].as_bool()` at `apple.rs:283` with `coerce_bool(&claims["email_verified"])`.
- [ ] Set `is_private_email: coerce_bool(&claims["is_private_email"])` in the `IdentityClaims` constructor at `apple.rs:280-289` (replacing the `None` placeholder task 02 left).
- [ ] Add tests in the `apple.rs` module: token missing `aud` rejected, token missing `iss` rejected, future-`nbf` token rejected, string `email_verified: "true"` mapped to `Some(true)`, and both a string `is_private_email: "true"` and a bool `is_private_email: true` mapped to `Some(true)`.
- [ ] Apply the change spec's §"Tiers, Tier 2 Apple" block to `.specs/service/specs/05-provider-system.md`: add the Apple bool-or-string coercion note to the Tier 2 Apple description (coercion of `email_verified` and `is_private_email`, with `is_private_email` surfaced as a first-class `Option<bool>` populated only by the Apple provider). Bump the page's `**Date:**` to `2026-07-02`. (Task 03 applies the §"OidcProvider behaviour" and §Decisions blocks to different sections of the same page and sets the same `**Date:**` value — the two edits merge cleanly.)
- [ ] Ensure the touched functions carry at least two meaningful assertions and any new bound is a named constant.

## Definition of done

- [ ] An ID token omitting `iss` or `aud`, and a token whose `nbf` is in the future, are each rejected with `Error::InvalidGrant`; a well-formed Apple token still validates.
- [ ] A string `email_verified: "true"` maps to `Some(true)` (so the registration domain allowlist admits the Apple sign-in), and both string and bool `is_private_email` map to `Some(_)`.
- [ ] Negative-space tests cover each new rejection path (missing `aud`, missing `iss`, future `nbf`), and the coercion cases above are asserted.
- [ ] `.specs/service/specs/05-provider-system.md` §"Tiers, Tier 2 Apple" describes the bool-or-string coercion of `email_verified`/`is_private_email` and the first-class `is_private_email` field, and the page `**Date:**` is bumped — moved together with this code change.
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: a reviewer runs the `apple.rs` tests and sees the missing-`iss`/`aud`, future-`nbf`, string-`email_verified`, and string/bool-`is_private_email` cases all behave as specified, and confirms 05-provider-system.md §"Tiers, Tier 2 Apple" now carries the Apple coercion note.
