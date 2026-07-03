# Done Certificate — Task 03: ECDSA DER→raw conversion on sign

**Task:** [03-ecdsa_der_to_raw_sign.md](03-ecdsa_der_to_raw_sign.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

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
  - *Status:* ☑ SATISFIED — `sign` (`crates/adapters/src/kms/mod.rs:257-297`) routes through `signature_to_jws_form` → `der_to_raw_ecdsa`; the ES\* branches call the matching curve's `p256/p384/p521::ecdsa::Signature::from_der(..)` (mod.rs:112/117/122) followed by `.to_vec()` — the same fixed-width big-endian `r || s` bytes as `to_bytes()`, asserted against the named width constants. Per-curve tests `test_der_to_raw_ecdsa_es{256,384,512}_round_trips_and_is_fixed_width` PASS, asserting exact widths 64/96/132 and round-tripping `r`/`s` against `split_bytes()`. RS\*/PS\* pass through `Ok(kms_signature)` byte-identical (`test_signature_to_jws_form_passes_rsa_and_pss_through_unchanged` PASS for all six algorithms). Resolution: `from_der` resolves to the curve-specific `ecdsa::Signature<NistP*>` re-export in each crate — no hand-rolled ASN.1, no shadowing. `to_der` appears only in test fixtures (mod.rs:584/605/626/684), never in `sign` — no raw→DER path.

- **O2 — Negative-space test: malformed DER returns KeyError.**
  - *Claim:* a malformed DER ECDSA signature fed to the conversion returns `Err(KeyError)`, not a panic or truncated buffer.
  - *Evidence to collect:* run the malformed-DER test — expect `KeyError`. Trace the `from_der` error mapping and confirm it maps to `Error::KeyError` (no `unwrap`).
  - *Status:* ☑ SATISFIED — `test_der_to_raw_ecdsa_malformed_der_is_key_error` (garbage bytes, ES256) and `test_der_to_raw_ecdsa_truncated_der_is_key_error` (truncated SEQUENCE, ES384) both PASS, matching `Err(Error::KeyError { .. })`. Trace: each `from_der` failure is mapped via `.map_err(|e| Error::KeyError { detail: ... })` (mod.rs:113-115/118-120/123-125); no `unwrap` on the parse path.

- **O3 — Widths are named constants; ≥2 assertions; ES\* branch exhaustive.**
  - *Claim:* 64/96/132 are named constants, `sign` carries at least two meaningful assertions, and the ES\* match is exhaustive over the accepted algorithms.
  - *Evidence to collect:* read `sign` and grep for the width literals; confirm they are named. Count assertions in the conversion tests.
  - *Status:* ☑ SATISFIED — widths are named constants `RAW_SIG_LEN_ES256`/`RAW_SIG_LEN_ES384`/`RAW_SIG_LEN_ES512` (= 64/96/132, mod.rs:84-92) reached via `ecdsa_raw_signature_len`; no bare width literals in `sign`. `sign` carries two assertions (non-empty KMS signature, mod.rs:280-283; converted length equals the curve width, mod.rs:287-294) and `der_to_raw_ecdsa` carries a third length assertion. The ES\* branch is exhaustive: `signature_to_jws_form` matches `ES256|ES384|ES512`, all six `RS*`/`PS*`, and a catch-all arm returning `KeyError`; `der_to_raw_ecdsa` likewise rejects non-ECDSA algorithms with `KeyError`. Conversion tests carry 3 assertions per curve (width + r + s).

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests, lint, and format are clean.
  - *Evidence to collect:* run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` (per `.specs/development-guidelines.md` §Definition of done) — expect all clean.
  - *Status:* ☑ SATISFIED — `cargo fmt --check` clean (no output); `cargo clippy --workspace -- -D warnings` clean; `cargo nextest run --workspace` → 202 passed, 2 skipped (ignored KMS/LocalStack integration tests), 0 failed.

- **O5 — Reviewable: per-curve DER→raw tests assert fixed-width output.**
  - *Claim:* a reviewer runs the kms tests and sees the per-curve conversion tests assert 64/96/132-byte output and pass.
  - *Evidence to collect:* run `cargo nextest run -p oidc-exchange-adapters kms::tests` — expect the DER→raw conversion tests to PASS.
  - *Status:* ☑ SATISFIED — ran `cargo nextest run -p oidc-exchange-adapters kms::tests`: 16/16 PASS, including the three per-curve fixed-width round-trip tests, the two malformed/truncated-DER `KeyError` tests, and the RS\*/PS\* byte-identical pass-through test.

## Regression check

- Any caller of `KmsKeyManager::sign` with an RS256/PS256 configuration → still receives the unmodified KMS signature bytes : ☑ PRESERVED — trace: `sign` → `signature_to_jws_form("RS256"|"PS256", sig)` → `Ok(kms_signature)` (identity, no copy mutation); `ecdsa_raw_signature_len` returns `None` for RS\*/PS\* so the width assertion is skipped; `test_signature_to_jws_form_passes_rsa_and_pss_through_unchanged` asserts byte-identity for all six RS\*/PS\* algorithms and passes.
- The `signing_algorithm()` mapping and the KMS Sign request builder are unchanged → `test_signing_algorithm_mapping` still passes : ☑ PRESERVED — the diff touches nothing in `signing_algorithm()` or the `.sign().key_id(..).signing_algorithm(..).message_type(Raw)` request builder (mod.rs:260-268 unchanged); `test_signing_algorithm_mapping` PASS.

## Residue

- Outside the DoD: end-to-end verification of a minted token by a third-party JWT library is exercised at the local-verify task (04) and any server E2E, not here; this task asserts the wire-form width and round-trip only.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O5 all SATISFIED with direct evidence — per-curve `from_der` conversion with named width constants and two `sign` assertions is in place, malformed DER maps to `KeyError`, fmt/clippy/nextest are clean (202 workspace tests, 16 kms tests pass), and both regression traces (RS\*/PS\* byte-identity, untouched Sign request builder) are PRESERVED; the only deviation from the authored evidence text is `.to_vec()` in place of `.to_bytes()`, which yields the same fixed-width `r || s` bytes as the round-trip tests prove.
