# Done Certificate — Task 01: RSA JWK Base64urlUInt encoding

**Task:** [01-rsa_jwk_base64urluint.md](01-rsa_jwk_base64urluint.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

> This certificate is a verification protocol for Task 01. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 01) ≡ every obligation O1…O5 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** RSA JWKs at `/keys` follow RFC 7518 §6.3 Base64urlUInt — no leading zero octets in `n`/`e`, so `e = 65537` encodes as `AQAB`.
- **P2 — Obligations.** The task is done iff O1…O5 all hold; O5 is the Reviewable item.
- **P3 — Invariants.** Must not break the RSA arm's `n` encoding for a genuine multi-byte modulus, nor the ES256/ES384 EC arm of `parse_spki_to_jwk` (untouched by this task).

## Obligations

- **O1 — Generated RSA key produces `e == "AQAB"` and no leading zero octets in `n`/`e`.**
  - *Claim:* `parse_spki_to_jwk(spki, "RS256", kid)` for a 2048-bit key yields `jwk["e"] == "AQAB"`, and neither `n` nor `e` decodes to bytes beginning with `0x00`.
  - *Evidence to collect:* read `crates/adapters/src/kms/mod.rs` around the RSA `n`/`e` encoding (was `:89-90`) and confirm the `to_be_bytes()` output is passed through a leading-zero strip before base64url. Run the RSA JWK test in `kms::tests` (`test_parse_rsa_public_key_to_jwk` or its extension) — expect PASS with an explicit `assert_eq!(jwk["e"], "AQAB")`.
  - *Checks:* resolve the encode helper called for `e` — confirm it is the new strip-and-encode helper, not a bare `URL_SAFE_NO_PAD.encode(...)` still including leading zeros.
  - *Status:* ☑ SATISFIED — `crates/adapters/src/kms/mod.rs:105-106` pass `to_be_bytes()` output for both `n` and `e` through `base64url_uint(...)` (defined at `:86-92`, the only definition in the crate — resolves to the new helper, no shadowing, no bare `URL_SAFE_NO_PAD.encode` left in the RSA arm). Ran `cargo nextest run -p oidc-exchange-adapters kms::tests` → `test_parse_rsa_public_key_to_jwk` PASS with explicit `assert_eq!(jwk["e"], "AQAB", ...)` (mod.rs:358-361) and a decoded-`n` first-byte `!= 0x00` assertion (mod.rs:366-372).

- **O2 — Negative-space test: encoder strips leading zeros but preserves a byte for a zero value.**
  - *Claim:* the strip-and-encode helper removes leading `0x00` octets yet never returns an empty string (a zero-valued input encodes to a single-byte, not empty).
  - *Evidence to collect:* run the helper's unit test with an input carrying a leading zero byte and with a zero value — expect the leading-zero case to drop the `0x00` and the zero-value case to yield a non-empty result. Trace the helper and confirm the guard that keeps one byte for zero.
  - *Status:* ☑ SATISFIED — `test_base64url_uint_strips_leading_zeros_but_not_the_value` (mod.rs:376-396) PASS: `[0x00,0x01,0x00,0x01]` → `"AQAB"`, `[0x00,0x00,0x2a]` → `"Kg"`, and all-zero `[0x00,0x00,0x00]` → non-empty, equal to the encoding of a single `0x00` octet. Trace of `base64url_uint` (mod.rs:87-90): `position(|&b| b != 0)` returning `None` falls to the `ZERO_VALUE_OCTET` branch — the guard that keeps one byte for zero.

- **O3 — Touched arm/helper carry ≥2 assertions; any width is a named constant.**
  - *Claim:* the RSA arm and the new helper each have at least two meaningful assertions across their tests, and no bare numeric width literal is introduced.
  - *Evidence to collect:* read the test(s) covering the RSA arm and the helper; count the assertions on `e`, `n`, and emptiness. Grep the helper for numeric literals.
  - *Status:* ☑ SATISFIED — RSA-arm test carries 6+ assertions (`kty`, `alg`, `kid`, `n` present, `e == "AQAB"`, decoded `n` has no leading `0x00`); helper test carries 4 assertions (leading-zero strip → `"AQAB"`, multi-zero strip → `"Kg"`, non-empty on zero value, single-zero-octet equivalence). Grep of the helper body: the only numeric literal is the `b != 0` zero-octet comparison (not a width); the preserved zero byte is the named constant `ZERO_VALUE_OCTET` (mod.rs:80). No magic width introduced.

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests, lint, and format are clean for the adapters crate.
  - *Evidence to collect:* run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, and `cargo nextest run --workspace` (per `.specs/development-guidelines.md` §Definition of done) — expect all clean.
  - *Status:* ☑ SATISFIED — `cargo fmt --check` clean (no output); `cargo clippy --workspace -- -D warnings` finished with no warnings; `cargo nextest run --workspace` → 192 tests run, 192 passed, 2 skipped (pre-existing ignored tests, unrelated to this task).

- **O5 — Reviewable: RSA JWK test asserting `e == "AQAB"` passes.**
  - *Claim:* a reviewer runs the adapter's kms tests and sees the RSA `e == "AQAB"` assertion pass.
  - *Evidence to collect:* run `cargo nextest run -p oidc-exchange-adapters kms::tests` — expect the RSA JWK test to PASS with the `AQAB` assertion.
  - *Status:* ☑ SATISFIED — ran `cargo nextest run -p oidc-exchange-adapters kms::tests`: 6 tests run, 6 passed, including `kms::tests::test_parse_rsa_public_key_to_jwk` (carries the `assert_eq!(jwk["e"], "AQAB", ...)`) and `kms::tests::test_base64url_uint_strips_leading_zeros_but_not_the_value`.

## Regression check

- `KmsKeyManager::fetch_public_jwk` calls `parse_spki_to_jwk` for RSA algorithms → after the change a real RSA SPKI still yields a valid `n` (multi-byte, unchanged aside from any stripped leading zero) : ☑ PRESERVED — `test_parse_rsa_public_key_to_jwk` generates a real 2048-bit key and PASSes: `n` is present, base64url-decodes cleanly, and its first byte is nonzero (a 2048-bit modulus has its top bit set, so stripping is a no-op on the value itself).
- The ES256/ES384 EC arm of `parse_spki_to_jwk` is untouched → its existing test still passes : ☑ PRESERVED — the jj diff touches only the RSA arm, the new helper, and tests; `kms::tests::test_parse_ec_public_key_to_jwk` PASS.

## Residue

- Outside the DoD: RFC 7638 thumbprint derivation itself is not implemented here; this task only makes `n`/`e` thumbprint-correct. Not an obligation of Task 01.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O5 all SATISFIED with direct evidence (helper `base64url_uint` at mod.rs:86-92 wired into both `n` and `e`, targeted kms tests 6/6 PASS with the `AQAB` assertion, fmt/clippy/full workspace suite clean at 192/192), and both regression surfaces (RSA `n` for a real 2048-bit key, untouched EC arm) are PRESERVED.
