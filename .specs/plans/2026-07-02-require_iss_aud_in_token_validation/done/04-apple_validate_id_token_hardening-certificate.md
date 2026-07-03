# Done Certificate — Task 04: harden Apple validate_id_token

**Task:** [04-apple_validate_id_token_hardening.md](04-apple_validate_id_token_hardening.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

> This certificate is a verification protocol for Task 04. A validating agent discharges it: for
> each obligation, collect the named evidence, run the named checks, set the Status, then derive
> the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its evidence; do
> not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 04) ≡ every obligation O1…O6 below holds, each backed by the evidence the obligation
names — not by assertion.

## Premises

- **P1 — Goal.** The Apple provider rejects `iss`/`aud`-omitting and future-`nbf` tokens, coerces
  `email_verified` via the shared helper, and populates `is_private_email` from bool-or-string coercion;
  and the `05-provider-system.md` §"Tiers, Tier 2 Apple" prose gains the Apple coercion note alongside
  the code change.
- **P2 — Obligations.** Done iff O1…O6 all hold; O6 is the Reviewable item.
- **P3 — Invariants.** Must not break the existing Apple valid-token path in `validate_id_token`
  (`crates/providers/src/apple.rs`), the existing alg-reject at `:249-259`, the `JwksCache` fetch,
  or the `sub` extraction.

## Obligations

- **O1 — Tokens omitting `iss`/`aud` and future-`nbf` tokens are rejected; a valid Apple token still validates.**
  - *Claim:* `validate_id_token` returns `Error::InvalidGrant` when the token has no `aud`, no `iss`, or a future `nbf`, and succeeds for a well-formed token.
  - *Evidence to collect:* read `crates/providers/src/apple.rs` around the `Validation` build (`:260-262`); confirm `set_required_spec_claims(&["exp","iss","aud"])` and `validate_nbf = true` are set. Run the new `apple.rs` tests for missing-`aud`, missing-`iss`, and future-`nbf` — expect each to assert an `Err`, and the positive test `Ok`.
  - *Checks:* resolve `set_required_spec_claims`/`validate_nbf` to the `jsonwebtoken::Validation` API.
  - *Status:* ✅ SATISFIED — `apple.rs:264-265` sets `validation.set_required_spec_claims(&["exp", "iss", "aud"])` and `validation.validate_nbf = true` on the `Validation::new(jwk_alg)` built at `:261`; `validation` is `jsonwebtoken::Validation` (imported at `apple.rs:5-7`), so both resolve to the jsonwebtoken API — no shadow. Tests `validate_id_token_rejects_missing_aud`, `validate_id_token_rejects_missing_iss`, `validate_id_token_rejects_future_nbf` each PASS asserting `Err` + `matches!(…, Error::InvalidGrant { .. })` (decode errors map to `Error::InvalidGrant` at `apple.rs:268-271`); positive path `exchange_and_validate_flow` (well-formed token with `iss`/`aud`) PASS.

- **O2 — String `email_verified` maps to `Some(true)` and both string/bool `is_private_email` map to `Some(_)`.**
  - *Claim:* `email_verified: "true"` → `Some(true)` (so the allowlist admits the sign-in), and `is_private_email` as `"true"` or `true` → `Some(true)`.
  - *Evidence to collect:* confirm `apple.rs:283` now calls `coerce_bool(&claims["email_verified"])` and the constructor at `:280-289` sets `is_private_email: coerce_bool(&claims["is_private_email"])`. Run the string-`email_verified` and string/bool-`is_private_email` tests — expect `Some(true)`.
  - *Checks:* resolve `coerce_bool` to `oidc_exchange_adapters::shared::claims::coerce_bool` (task 01), not a local.
  - *Status:* ✅ SATISFIED — `apple.rs:286` is `email_verified: coerce_bool(&claims["email_verified"])` and `:288` is `is_private_email: coerce_bool(&claims["is_private_email"])`. `coerce_bool` resolves via the import at `apple.rs:8` (`use oidc_exchange_adapters::shared::claims::coerce_bool;`) to the shared helper at `crates/adapters/src/shared/claims.rs:14`; no local `fn coerce_bool` exists in the providers crate (grep confirmed). Tests `validate_id_token_coerces_string_email_verified` (asserts `Some(true)`), `validate_id_token_coerces_string_is_private_email`, and `validate_id_token_coerces_bool_is_private_email` (both assert `Some(true)`) all PASS.

- **O3 — Negative-space tests cover each new rejection path and the coercion cases are asserted.**
  - *Claim:* tests exist for missing `aud`, missing `iss`, future `nbf`, plus the `email_verified`/`is_private_email` coercions.
  - *Evidence to collect:* enumerate the new tests in the `apple.rs` test module (using `generate_es256_test_keys`); confirm each rejection path asserts `Err` and each coercion asserts the expected `Some(_)`.
  - *Status:* ✅ SATISFIED — six new tests in the `apple.rs` module, all built on `generate_es256_test_keys` via the `provider_with_mock_jwks`/`sign_id_token` helpers: `validate_id_token_rejects_missing_aud`, `validate_id_token_rejects_missing_iss`, `validate_id_token_rejects_future_nbf` (each asserts `is_err()` and `Error::InvalidGrant` — 2 assertions each), `validate_id_token_coerces_string_email_verified` (asserts `email_verified == Some(true)` and email round-trip), `validate_id_token_coerces_string_is_private_email` and `validate_id_token_coerces_bool_is_private_email` (each asserts `is_private_email == Some(true)` and `subject` — 2 assertions each). All 6 PASS under nextest.

- **O4 — `05-provider-system.md` §"Tiers, Tier 2 Apple" gains the Apple-coercion note alongside the code.**
  - *Claim:* the Tier 2 Apple description carries the bool-or-string coercion note for `email_verified` and `is_private_email`, states `is_private_email` is a first-class `Option<bool>` populated only by the Apple provider, and the page `**Date:**` is bumped.
  - *Evidence to collect:* read `.specs/service/specs/05-provider-system.md`; confirm the Tier 2 Apple coercion note matches the change spec's §"Tiers, Tier 2 Apple" Proposed-changes block, and that `**Date:**` was bumped from `2026-06-24`.
  - *Checks:* confirm the §"OidcProvider behaviour" + §Decisions *Required spec claims* edits (task 03's blocks) and the `**Date:**` value are consistent — both tasks set the same date; a divergent date or a missing behaviour/Decision block signals a merge that dropped one edit.
  - *Status:* ✅ SATISFIED — the Tier 2 Apple description now carries the coercion paragraph ("Apple's ID tokens sometimes carry `email_verified` (and `is_private_email`) as the JSON strings…"), states the bool-or-string coercion onto `IdentityClaims.email_verified`/`is_private_email`, and describes `is_private_email` as a first-class `Option<bool>` populated only by the Apple provider (generic OIDC leaves it `None`). Page `**Date:**` is `2026-07-02` (bumped from 2026-06-24). Consistency check: task 03's blocks are present — §"OidcProvider behaviour" `nbf`-when-present bullet (line 65) and §Decisions *Required spec claims* entry (line 105) — with the same Date; nothing dropped in the merge.

- **O5 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean, new bounds named, ≥2 assertions per touched function.
  - *Evidence to collect:* run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, and `cargo nextest run --workspace` (from `.specs/development-guidelines.md` §Definition of done) — expect all clean.
  - *Status:* ✅ SATISFIED — `cargo fmt --check` clean (exit 0); `cargo clippy --workspace -- -D warnings` finished with no warnings; `cargo nextest run --workspace` → 236 tests run, 236 passed, 10 skipped. New bound is the named constant `TEST_FUTURE_NBF_OFFSET_SECS` (apple.rs test module); every new test carries ≥2 assertions.

- **O6 — Reviewable: the Apple tests show every new case behaves as specified (Reviewable).**
  - *Claim:* a reviewer runs the `apple.rs` tests and sees the missing-`iss`/`aud`, future-`nbf`, string-`email_verified`, and string/bool-`is_private_email` cases behave as specified, and confirms 05-provider-system.md §"Tiers, Tier 2 Apple" carries the Apple coercion note.
  - *Evidence to collect:* run `cargo nextest run -p oidc-exchange-providers` filtered to the `apple` tests; observe each named case PASS; open 05-provider-system.md and confirm the Apple coercion note.
  - *Status:* ✅ SATISFIED — exercised as named: `cargo nextest run -p oidc-exchange-providers -E 'test(apple)'` → 14 tests run, 14 passed, including the missing-`iss`, missing-`aud`, future-`nbf`, string-`email_verified`, and string/bool-`is_private_email` cases; opened `.specs/service/specs/05-provider-system.md` and confirmed the Tier 2 Apple coercion note is present.

## Regression check

- `crates/core/src/service/exchange.rs` reads `claims.email_verified` from the Apple provider's result; trace that a bool `email_verified: true` still maps to `Some(true)` via `coerce_bool` so the allowlist path is unchanged and that an Apple sign-in with `"email_verified": "true"` now passes where it previously failed : ✅ PRESERVED — `exchange.rs:104` checks `claims.email_verified != Some(true)`; `coerce_bool` (claims.rs:14-17) returns `value.as_bool()` first, so a JSON bool `true` still maps to `Some(true)` exactly as `.as_bool()` did (confirmed by `exchange_and_validate_flow`, which uses `"email_verified": true` and PASSES), and the string `"true"` now also yields `Some(true)` (confirmed by `validate_id_token_coerces_string_email_verified`), so the allowlist now admits the previously-denied Apple sign-in.
- Existing `apple.rs` valid-token tests (with `iss`/`aud` present) still pass unchanged : ✅ PRESERVED — all 8 pre-existing `apple.rs` tests (incl. `exchange_and_validate_flow`, `revoke_token_posts_with_client_secret`, client-secret and from_config tests) PASS unchanged; full workspace suite 236/236 green.

## Residue

- Apple's alg path already errors on missing/unrecognised `alg` (`apple.rs:249-259`); no alg-inference change is in scope for this task (unlike task 03). Not an obligation.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O6 all SATISFIED with collected evidence (code read at apple.rs:264-265/:286/:288, `coerce_bool` resolved to the shared adapters helper with no shadow, all 6 new tests plus the full 236-test workspace suite PASS, fmt/clippy clean, and 05-provider-system.md carries the Tier 2 Apple coercion note with Date 2026-07-02 alongside task 03's intact blocks); both regression traces PRESERVED, so the verdict derives as DONE.
