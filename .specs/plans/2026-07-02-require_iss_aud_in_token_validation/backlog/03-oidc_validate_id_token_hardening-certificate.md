# Done Certificate — Task 03: harden OIDC validate_id_token

**Task:** [03-oidc_validate_id_token_hardening.md](03-oidc_validate_id_token_hardening.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified

> This certificate is a verification protocol for Task 03. A validating agent discharges it: for
> each obligation, collect the named evidence, run the named checks, set the Status, then derive
> the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its evidence; do
> not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 03) ≡ every obligation O1…O6 below holds, each backed by the evidence the obligation
names — not by assertion.

## Premises

- **P1 — Goal.** The generic OIDC adapter rejects `iss`/`aud`-omitting and future-`nbf` tokens,
  infers the alg from the JWK's `kty`/`crv` when `alg` is absent, and coerces `email_verified`;
  and the `05-provider-system.md` §"OidcProvider behaviour" + §Decisions *Required spec claims*
  prose is moved to its merged form alongside the code change.
- **P2 — Obligations.** Done iff O1…O6 all hold; O6 is the Reviewable item.
- **P3 — Invariants.** Must not break the existing valid-token path in `validate_id_token`
  (`crates/adapters/src/oidc/mod.rs`), the `JwksCache` fetch/decode, the `sub` extraction, or the
  behaviour for JWKs that *do* carry a recognised `alg` string.

## Obligations

- **O1 — Tokens omitting `iss`/`aud` and future-`nbf` tokens are rejected; a valid token still validates.**
  - *Claim:* `validate_id_token` returns `Error::InvalidGrant` when the ID token has no `aud`, no `iss`, or a future `nbf`, and succeeds for a well-formed token.
  - *Evidence to collect:* read `crates/adapters/src/oidc/mod.rs` around the `Validation` build (`:137-139`); confirm `set_required_spec_claims(&["exp","iss","aud"])` and `validate_nbf = true` are set. Run the new `oidc/mod.rs` tests for missing-`aud`, missing-`iss`, and future-`nbf` — expect each to assert an `Err`, and the positive test to assert `Ok`.
  - *Checks:* resolve `set_required_spec_claims`/`validate_nbf` to the `jsonwebtoken::Validation` API, not a local.
  - *Status:* ☐ unverified

- **O2 — Alg-less RSA validates as RS256, alg-less EC P-256 as ES256, unrecognised alg-less key rejected.**
  - *Claim:* when the matched JWK carries no `alg`, the algorithm is inferred from `kty` (`RSA`→RS256, `EC`+`crv` P-256→ES256 / P-384→ES384, `OKP`→EdDSA); any other alg-less key returns `Error::InvalidGrant`.
  - *Evidence to collect:* read the replacement for the `.unwrap_or(Algorithm::RS256)` at `oidc/mod.rs:136`; confirm the `kty`/`crv` branches and the error fallback (mirroring `apple.rs:249-259`). Run the alg-less RSA and alg-less EC tests — expect PASS; confirm a test asserts an unrecognised alg-less key is rejected.
  - *Checks:* confirm a JWK with a recognised `alg` string still uses that `alg` (the inference is only the absent-`alg` branch).
  - *Status:* ☐ unverified

- **O3 — Negative-space tests cover each new rejection path and string `email_verified` maps to `Some(true)`.**
  - *Claim:* tests exist for missing `aud`, missing `iss`, future `nbf`, unrecognised alg-less key, and a `email_verified: "true"` string mapping to `Some(true)`.
  - *Evidence to collect:* enumerate the new tests in the `oidc/mod.rs` test module (using `generate_rsa_test_keys`); confirm the `email_verified` line at `:160` now calls `coerce_bool(&claims["email_verified"])` and that a string-`"true"` test asserts `Some(true)`.
  - *Checks:* resolve `coerce_bool` to `oidc_exchange_adapters::shared::claims::coerce_bool` (task 01), not a local helper.
  - *Status:* ☐ unverified

- **O4 — `05-provider-system.md` prose moved to its merged form alongside the code.**
  - *Claim:* §"OidcProvider behaviour" replaces the single `validate_id_token` bullet with the two-bullet merged form (required `exp`/`iss`/`aud` presence via `set_required_spec_claims`, `nbf`-when-present, and JWK `kty`/`crv` alg inference with the alg-less-key reject), §Decisions carries the *Required spec claims* entry, and the page `**Date:**` is bumped.
  - *Evidence to collect:* read `.specs/service/specs/05-provider-system.md`; confirm the §"OidcProvider behaviour" bullets and the §Decisions *Required spec claims* entry match the change spec's Proposed-changes blocks, and that `**Date:**` was bumped from `2026-06-24`.
  - *Checks:* confirm the §"Tiers, Tier 2 Apple" Apple-coercion note (task 04's block) and the `**Date:**` value are consistent — both tasks set the same date; a divergent date or a missing Apple note signals a merge that dropped one edit.
  - *Status:* ☐ unverified

- **O5 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean, new bounds named, ≥2 assertions per touched function.
  - *Evidence to collect:* run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, and `cargo nextest run --workspace` (from `.specs/development-guidelines.md` §Definition of done) — expect all clean.
  - *Status:* ☐ unverified

- **O6 — Reviewable: the OIDC tests show every new case behaves as specified (Reviewable).**
  - *Claim:* a reviewer runs the `oidc/mod.rs` tests and sees the missing-`iss`/`aud`, future-`nbf`, alg-less RSA/EC, and string-`email_verified` cases behave as specified, and confirms 05-provider-system.md §"OidcProvider behaviour" + §Decisions *Required spec claims* match the merged form.
  - *Evidence to collect:* run `cargo nextest run -p oidc-exchange-adapters` filtered to the `oidc` tests; observe each named case PASS; open 05-provider-system.md and confirm the behaviour bullets and the Decision.
  - *Status:* ☐ unverified

## Regression check

- `crates/core/src/service/exchange.rs` calls the `IdentityProvider::validate_id_token` port and reads `claims.email_verified`; trace that a bool `email_verified: true` still maps to `Some(true)` through `coerce_bool` so the allowlist path is unchanged : ☐ (PRESERVED / REGRESSION)
- Existing `oidc/mod.rs` tests that validated a well-formed token (with `iss`/`aud` present) still pass unchanged : ☐ (PRESERVED / REGRESSION)

## Residue

- The RSA-without-`alg`→RS256 choice is a deliberate decision (plan.md, change spec): RS/PS families are indistinguishable from key parameters and RS256 matches Azure AD. Not a defect to flag.

## Conclusion

VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
