# Done Certificate — Task 03: harden OIDC validate_id_token

**Task:** [03-oidc_validate_id_token_hardening.md](03-oidc_validate_id_token_hardening.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

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
  - *Status:* ✅ SATISFIED — `crates/adapters/src/oidc/mod.rs:164-165` sets `validation.set_required_spec_claims(&["exp", "iss", "aud"])` and `validation.validate_nbf = true` on the `Validation` built at `:161`. `validation` is `jsonwebtoken::Validation` (`use jsonwebtoken::{…, Validation}` at `mod.rs:2`); no local shadow. Tests `validate_id_token_rejects_missing_aud`, `validate_id_token_rejects_missing_iss`, `validate_id_token_rejects_future_nbf` all PASS asserting `Error::InvalidGrant`; positive test `validate_id_token_succeeds_for_valid_jwt` PASSes unchanged.

- **O2 — Alg-less RSA validates as RS256, alg-less EC P-256 as ES256, unrecognised alg-less key rejected.**
  - *Claim:* when the matched JWK carries no `alg`, the algorithm is inferred from `kty` (`RSA`→RS256, `EC`+`crv` P-256→ES256 / P-384→ES384, `OKP`→EdDSA); any other alg-less key returns `Error::InvalidGrant`.
  - *Evidence to collect:* read the replacement for the `.unwrap_or(Algorithm::RS256)` at `oidc/mod.rs:136`; confirm the `kty`/`crv` branches and the error fallback (mirroring `apple.rs:249-259`). Run the alg-less RSA and alg-less EC tests — expect PASS; confirm a test asserts an unrecognised alg-less key is rejected.
  - *Checks:* confirm a JWK with a recognised `alg` string still uses that `alg` (the inference is only the absent-`alg` branch).
  - *Status:* ✅ SATISFIED — the `.unwrap_or(Algorithm::RS256)` is replaced by `.map(Ok).unwrap_or_else(|| infer_alg_from_jwk(jwk))?` at `mod.rs:159-160`; `infer_alg_from_jwk` (`mod.rs:32-45`) matches `(kty, crv)`: RSA→RS256, EC/P-256→ES256, EC/P-384→ES384, OKP→EdDSA, else `Error::InvalidGrant` — mirroring `apple.rs:249-259`. Tests `validate_id_token_alg_less_rsa_jwk_infers_rs256`, `validate_id_token_alg_less_ec_p256_jwk_infers_es256` PASS (fixtures assert the JWK omits `alg`); `validate_id_token_rejects_unrecognised_alg_less_key` (alg-less `kty: oct`) asserts `Error::InvalidGrant`, PASS. Recognised-`alg` branch: a present, recognised `alg` string maps to `Some(Algorithm)` before the `unwrap_or_else`, so inference never runs — the pre-existing tests whose JWKs carry `alg: RS256` (e.g. `validate_id_token_succeeds_for_valid_jwt`) still PASS.

- **O3 — Negative-space tests cover each new rejection path and string `email_verified` maps to `Some(true)`.**
  - *Claim:* tests exist for missing `aud`, missing `iss`, future `nbf`, unrecognised alg-less key, and a `email_verified: "true"` string mapping to `Some(true)`.
  - *Evidence to collect:* enumerate the new tests in the `oidc/mod.rs` test module (using `generate_rsa_test_keys`); confirm the `email_verified` line at `:160` now calls `coerce_bool(&claims["email_verified"])` and that a string-`"true"` test asserts `Some(true)`.
  - *Checks:* resolve `coerce_bool` to `oidc_exchange_adapters::shared::claims::coerce_bool` (task 01), not a local helper.
  - *Status:* ✅ SATISFIED — new tests enumerated in the `oidc/mod.rs` test module: `validate_id_token_rejects_missing_aud`, `validate_id_token_rejects_missing_iss`, `validate_id_token_rejects_future_nbf`, `validate_id_token_rejects_unrecognised_alg_less_key`, `validate_id_token_alg_less_rsa_jwk_infers_rs256`, `validate_id_token_alg_less_ec_p256_jwk_infers_es256`, `validate_id_token_coerces_string_email_verified` — all PASS (RSA cases use `generate_rsa_test_keys`). `mod.rs:186` now reads `email_verified: coerce_bool(&claims["email_verified"])`; `coerce_bool` resolves via `use crate::shared::claims::coerce_bool` (`mod.rs:8`) to the task-01 shared helper (`crates/adapters/src/shared/claims.rs:14`), no local shadow. The string test sends `email_verified: "true"` and asserts `identity.email_verified == Some(true)`; `is_private_email` stays `None`.

- **O4 — `05-provider-system.md` prose moved to its merged form alongside the code.**
  - *Claim:* §"OidcProvider behaviour" replaces the single `validate_id_token` bullet with the two-bullet merged form (required `exp`/`iss`/`aud` presence via `set_required_spec_claims`, `nbf`-when-present, and JWK `kty`/`crv` alg inference with the alg-less-key reject), §Decisions carries the *Required spec claims* entry, and the page `**Date:**` is bumped.
  - *Evidence to collect:* read `.specs/service/specs/05-provider-system.md`; confirm the §"OidcProvider behaviour" bullets and the §Decisions *Required spec claims* entry match the change spec's Proposed-changes blocks, and that `**Date:**` was bumped from `2026-06-24`.
  - *Checks:* confirm the §"Tiers, Tier 2 Apple" Apple-coercion note (task 04's block) and the `**Date:**` value are consistent — both tasks set the same date; a divergent date or a missing Apple note signals a merge that dropped one edit.
  - *Status:* ✅ SATISFIED — §"OidcProvider behaviour" now carries the two-bullet merged form (required `exp`/`iss`/`aud` presence via `set_required_spec_claims` + match, `nbf`-when-present, and the JWK `kty`/`crv` inference with the alg-less-key reject), matching the change spec's Proposed-changes blocks verbatim; §Decisions carries the *Required spec claims* entry word-for-word; `**Date:**` bumped 2026-06-24 → 2026-07-02 in the same diff as the code. Check note: the §"Tiers, Tier 2 Apple" coercion note is absent, but task 04 is still in `backlog/` (not yet implemented) — this is the expected pre-task-04 state, not a dropped merge; the `**Date:**` value (2026-07-02) is the one task 04 will also set.

- **O5 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean, new bounds named, ≥2 assertions per touched function.
  - *Evidence to collect:* run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, and `cargo nextest run --workspace` (from `.specs/development-guidelines.md` §Definition of done) — expect all clean.
  - *Status:* ✅ SATISFIED — `cargo fmt --check` clean (no output); `cargo clippy --workspace --all-targets -- -D warnings` finished with no warnings; `cargo nextest run --workspace` → 230 tests run, 230 passed, 10 skipped. No new numeric bounds introduced in product code (`infer_alg_from_jwk` is a pure match); each new test carries ≥2 meaningful assertions.

- **O6 — Reviewable: the OIDC tests show every new case behaves as specified (Reviewable).**
  - *Claim:* a reviewer runs the `oidc/mod.rs` tests and sees the missing-`iss`/`aud`, future-`nbf`, alg-less RSA/EC, and string-`email_verified` cases behave as specified, and confirms 05-provider-system.md §"OidcProvider behaviour" + §Decisions *Required spec claims* match the merged form.
  - *Evidence to collect:* run `cargo nextest run -p oidc-exchange-adapters` filtered to the `oidc` tests; observe each named case PASS; open 05-provider-system.md and confirm the behaviour bullets and the Decision.
  - *Status:* ✅ SATISFIED — `cargo nextest run -p oidc-exchange-adapters -E 'test(oidc)'` → 16 tests run, 16 passed, including all seven new cases (missing-`aud`, missing-`iss`, future-`nbf`, alg-less RSA, alg-less EC P-256, unrecognised alg-less key, string `email_verified`) and the pre-existing positive/negative cases. Opened `05-provider-system.md`: the two behaviour bullets and the §Decisions *Required spec claims* entry match the change spec's merged form.

## Regression check

- `crates/core/src/service/exchange.rs` calls the `IdentityProvider::validate_id_token` port and reads `claims.email_verified`; trace that a bool `email_verified: true` still maps to `Some(true)` through `coerce_bool` so the allowlist path is unchanged : ✅ PRESERVED — `exchange.rs:104` tests `claims.email_verified != Some(true)`; `coerce_bool(&Value::Bool(true))` short-circuits on `value.as_bool()` (`shared/claims.rs:15-16`) → `Some(true)`, identical to the old `.as_bool()` mapping; all `oidc-exchange-core::exchange` tests pass in the workspace run.
- Existing `oidc/mod.rs` tests that validated a well-formed token (with `iss`/`aud` present) still pass unchanged : ✅ PRESERVED — `validate_id_token_succeeds_for_valid_jwt`, `validate_id_token_rejects_wrong_audience`, `validate_id_token_rejects_wrong_issuer`, `validate_id_token_rejects_expired_jwt` all PASS unmodified in the filtered run.

## Residue

- The RSA-without-`alg`→RS256 choice is a deliberate decision (plan.md, change spec): RS/PS families are indistinguishable from key parameters and RS256 matches Azure AD. Not a defect to flag.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O6 all SATISFIED with direct evidence (code read at the named lines, fmt/clippy clean, 230/230 workspace tests and all 16 filtered `oidc` tests passing, spec prose matching the change spec's merged form); both named regression surfaces PRESERVED, and the absent Tier-2 Apple note is the expected pre-task-04 state, not a dropped edit.
