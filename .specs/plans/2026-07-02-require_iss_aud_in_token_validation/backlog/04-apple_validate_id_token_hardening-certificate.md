# Done Certificate — Task 04: harden Apple validate_id_token

**Task:** [04-apple_validate_id_token_hardening.md](04-apple_validate_id_token_hardening.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified

> This certificate is a verification protocol for Task 04. A validating agent discharges it: for
> each obligation, collect the named evidence, run the named checks, set the Status, then derive
> the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its evidence; do
> not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 04) ≡ every obligation O1…O5 below holds, each backed by the evidence the obligation
names — not by assertion.

## Premises

- **P1 — Goal.** The Apple provider rejects `iss`/`aud`-omitting and future-`nbf` tokens, coerces
  `email_verified` via the shared helper, and populates `is_private_email` from bool-or-string coercion.
- **P2 — Obligations.** Done iff O1…O5 all hold; O5 is the Reviewable item.
- **P3 — Invariants.** Must not break the existing Apple valid-token path in `validate_id_token`
  (`crates/providers/src/apple.rs`), the existing alg-reject at `:249-259`, the `JwksCache` fetch,
  or the `sub` extraction.

## Obligations

- **O1 — Tokens omitting `iss`/`aud` and future-`nbf` tokens are rejected; a valid Apple token still validates.**
  - *Claim:* `validate_id_token` returns `Error::InvalidGrant` when the token has no `aud`, no `iss`, or a future `nbf`, and succeeds for a well-formed token.
  - *Evidence to collect:* read `crates/providers/src/apple.rs` around the `Validation` build (`:260-262`); confirm `set_required_spec_claims(&["exp","iss","aud"])` and `validate_nbf = true` are set. Run the new `apple.rs` tests for missing-`aud`, missing-`iss`, and future-`nbf` — expect each to assert an `Err`, and the positive test `Ok`.
  - *Checks:* resolve `set_required_spec_claims`/`validate_nbf` to the `jsonwebtoken::Validation` API.
  - *Status:* ☐ unverified

- **O2 — String `email_verified` maps to `Some(true)` and both string/bool `is_private_email` map to `Some(_)`.**
  - *Claim:* `email_verified: "true"` → `Some(true)` (so the allowlist admits the sign-in), and `is_private_email` as `"true"` or `true` → `Some(true)`.
  - *Evidence to collect:* confirm `apple.rs:283` now calls `coerce_bool(&claims["email_verified"])` and the constructor at `:280-289` sets `is_private_email: coerce_bool(&claims["is_private_email"])`. Run the string-`email_verified` and string/bool-`is_private_email` tests — expect `Some(true)`.
  - *Checks:* resolve `coerce_bool` to `oidc_exchange_adapters::shared::claims::coerce_bool` (task 01), not a local.
  - *Status:* ☐ unverified

- **O3 — Negative-space tests cover each new rejection path and the coercion cases are asserted.**
  - *Claim:* tests exist for missing `aud`, missing `iss`, future `nbf`, plus the `email_verified`/`is_private_email` coercions.
  - *Evidence to collect:* enumerate the new tests in the `apple.rs` test module (using `generate_es256_test_keys`); confirm each rejection path asserts `Err` and each coercion asserts the expected `Some(_)`.
  - *Status:* ☐ unverified

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean, new bounds named, ≥2 assertions per touched function.
  - *Evidence to collect:* run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, and `cargo nextest run --workspace` (from `.specs/development-guidelines.md` §Definition of done) — expect all clean.
  - *Status:* ☐ unverified

- **O5 — Reviewable: the Apple tests show every new case behaves as specified (Reviewable).**
  - *Claim:* a reviewer runs the `apple.rs` tests and sees the missing-`iss`/`aud`, future-`nbf`, string-`email_verified`, and string/bool-`is_private_email` cases behave as specified.
  - *Evidence to collect:* run `cargo nextest run -p oidc-exchange-providers` filtered to the `apple` tests; observe each named case PASS.
  - *Status:* ☐ unverified

## Regression check

- `crates/core/src/service/exchange.rs` reads `claims.email_verified` from the Apple provider's result; trace that a bool `email_verified: true` still maps to `Some(true)` via `coerce_bool` so the allowlist path is unchanged and that an Apple sign-in with `"email_verified": "true"` now passes where it previously failed : ☐ (PRESERVED / REGRESSION)
- Existing `apple.rs` valid-token tests (with `iss`/`aud` present) still pass unchanged : ☐ (PRESERVED / REGRESSION)

## Residue

- Apple's alg path already errors on missing/unrecognised `alg` (`apple.rs:249-259`); no alg-inference change is in scope for this task (unlike task 03). Not an obligation.

## Conclusion

VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
