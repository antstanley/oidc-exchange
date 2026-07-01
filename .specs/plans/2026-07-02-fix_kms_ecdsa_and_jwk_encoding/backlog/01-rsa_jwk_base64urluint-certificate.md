# Done Certificate — Task 01: RSA JWK Base64urlUInt encoding

**Task:** [01-rsa_jwk_base64urluint.md](01-rsa_jwk_base64urluint.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified

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
  - *Status:* ☐ unverified

- **O2 — Negative-space test: encoder strips leading zeros but preserves a byte for a zero value.**
  - *Claim:* the strip-and-encode helper removes leading `0x00` octets yet never returns an empty string (a zero-valued input encodes to a single-byte, not empty).
  - *Evidence to collect:* run the helper's unit test with an input carrying a leading zero byte and with a zero value — expect the leading-zero case to drop the `0x00` and the zero-value case to yield a non-empty result. Trace the helper and confirm the guard that keeps one byte for zero.
  - *Status:* ☐ unverified

- **O3 — Touched arm/helper carry ≥2 assertions; any width is a named constant.**
  - *Claim:* the RSA arm and the new helper each have at least two meaningful assertions across their tests, and no bare numeric width literal is introduced.
  - *Evidence to collect:* read the test(s) covering the RSA arm and the helper; count the assertions on `e`, `n`, and emptiness. Grep the helper for numeric literals.
  - *Status:* ☐ unverified

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests, lint, and format are clean for the adapters crate.
  - *Evidence to collect:* run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, and `cargo nextest run --workspace` (per `.specs/development-guidelines.md` §Definition of done) — expect all clean.
  - *Status:* ☐ unverified

- **O5 — Reviewable: RSA JWK test asserting `e == "AQAB"` passes.**
  - *Claim:* a reviewer runs the adapter's kms tests and sees the RSA `e == "AQAB"` assertion pass.
  - *Evidence to collect:* run `cargo nextest run -p oidc-exchange-adapters kms::tests` — expect the RSA JWK test to PASS with the `AQAB` assertion.
  - *Status:* ☐ unverified

## Regression check

- `KmsKeyManager::fetch_public_jwk` calls `parse_spki_to_jwk` for RSA algorithms → after the change a real RSA SPKI still yields a valid `n` (multi-byte, unchanged aside from any stripped leading zero) : ☐ (PRESERVED / REGRESSION)
- The ES256/ES384 EC arm of `parse_spki_to_jwk` is untouched → its existing test still passes : ☐ (PRESERVED / REGRESSION)

## Residue

- Outside the DoD: RFC 7638 thumbprint derivation itself is not implemented here; this task only makes `n`/`e` thumbprint-correct. Not an obligation of Task 01.

## Conclusion

VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
