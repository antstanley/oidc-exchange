# Done Certificate — Task 03: ECDSA DER→raw conversion on sign

**Task:** [03-ecdsa_der_to_raw_sign.md](03-ecdsa_der_to_raw_sign.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified

> This certificate is a verification protocol for Task 03. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 03) ≡ every obligation O1…O5 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** `sign` returns ES256/384/512 signatures as raw fixed-width `r || s` (64/96/132 bytes), the JWS wire form; RS\*/PS\* pass through unchanged.
- **P2 — Obligations.** The task is done iff O1…O5 all hold; O5 is the Reviewable item.
- **P3 — Invariants.** Must not change RS\*/PS\* `sign` output bytes; must not introduce a raw→DER conversion; the KMS Sign call itself stays intact.

## Obligations

- **O1 — ES\* conversion yields fixed-width raw `r || s`; RS\*/PS\* unchanged.**
  - *Claim:* for ES256/384/512 the post-Sign conversion produces exactly 64/96/132 bytes; for RS\*/PS\* the returned bytes equal the KMS response byte-for-byte.
  - *Evidence to collect:* read `crates/adapters/src/kms/mod.rs` `sign` (was `:150-175`) and confirm the ES\* branch calls the matching curve's `ecdsa::Signature::from_der(..)?.to_bytes()` and the length assertion. Run the per-curve conversion tests in `kms::tests` — expect PASS with exact-width assertions.
  - *Checks:* resolve `from_der`/`to_bytes` — confirm they are the `ecdsa::Signature` conversion for the correct curve (`p256`/`p384`/`p521`), not a hand-rolled or wrong-curve path; confirm no `to_der`/raw→DER call exists in `sign`.
  - *Status:* ☐ unverified

- **O2 — Negative-space test: malformed DER returns KeyError.**
  - *Claim:* a malformed DER ECDSA signature fed to the conversion returns `Err(KeyError)`, not a panic or truncated buffer.
  - *Evidence to collect:* run the malformed-DER test — expect `KeyError`. Trace the `from_der` error mapping and confirm it maps to `Error::KeyError` (no `unwrap`).
  - *Status:* ☐ unverified

- **O3 — Widths are named constants; ≥2 assertions; ES\* branch exhaustive.**
  - *Claim:* 64/96/132 are named constants, `sign` carries at least two meaningful assertions, and the ES\* match is exhaustive over the accepted algorithms.
  - *Evidence to collect:* read `sign` and grep for the width literals; confirm they are named. Count assertions in the conversion tests.
  - *Status:* ☐ unverified

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests, lint, and format are clean.
  - *Evidence to collect:* run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` (per `.specs/development-guidelines.md` §Definition of done) — expect all clean.
  - *Status:* ☐ unverified

- **O5 — Reviewable: per-curve DER→raw tests assert fixed-width output.**
  - *Claim:* a reviewer runs the kms tests and sees the per-curve conversion tests assert 64/96/132-byte output and pass.
  - *Evidence to collect:* run `cargo nextest run -p oidc-exchange-adapters kms::tests` — expect the DER→raw conversion tests to PASS.
  - *Status:* ☐ unverified

## Regression check

- Any caller of `KmsKeyManager::sign` with an RS256/PS256 configuration → still receives the unmodified KMS signature bytes : ☐ (PRESERVED / REGRESSION)
- The `signing_algorithm()` mapping and the KMS Sign request builder are unchanged → `test_signing_algorithm_mapping` still passes : ☐ (PRESERVED / REGRESSION)

## Residue

- Outside the DoD: end-to-end verification of a minted token by a third-party JWT library is exercised at the local-verify task (04) and any server E2E, not here; this task asserts the wire-form width and round-trip only.

## Conclusion

VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
