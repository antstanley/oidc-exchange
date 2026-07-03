# Task 03 — ECDSA DER→raw conversion on sign

**Plan:** [plan.md](../plan.md) · **Certificate:** [03-ecdsa_der_to_raw_sign-certificate.md](03-ecdsa_der_to_raw_sign-certificate.md)

**Implements:** [.specs/service/specs/02-ports-and-adapters.md](../../../service/specs/02-ports-and-adapters.md) §Port traits → KeyManager (`sign` returns signature bytes in the JWS wire form; ES\* on KMS converts DER `Ecdsa-Sig-Value` to raw `r || s`); change spec Implementation note 1.
**Depends on:** 02 (build — the P-521 curve type from the `p521` crate that task 02 adds is needed to convert ES512 signatures).
**Produces:** `sign` output for ES256/384/512 is the raw fixed-width `r || s` form JWS requires (64/96/132 bytes) instead of KMS's DER encoding, so tokens minted on KMS verify against the served JWKS with any standard JWT library. RSA and PSS signatures pass through unchanged.
**Pointers:** `crates/adapters/src/kms/mod.rs:150-175` (`sign`); the `ecdsa` crate reached via `p256`/`p384`/`p521` (`ecdsa::Signature::from_der(..)?.to_bytes()`).

## Steps

- [x] After the KMS Sign call in `sign`, branch on `self.algorithm`: for `ES256`/`ES384`/`ES512` parse the returned bytes as a DER `Ecdsa-Sig-Value` with the matching curve's `ecdsa::Signature::from_der` and emit `to_bytes()` (the fixed-width `r || s`); leave RS\*/PS\* bytes untouched.
- [x] Map a DER-parse failure to a domain `Error::KeyError` with detail, rather than unwrapping — no hand-rolled ASN.1.
- [x] Assert the converted length matches the curve's expected width (64/96/132) as a named-constant check before returning.
- [x] Add per-curve conversion tests: build a known DER `Ecdsa-Sig-Value` (or sign locally with a RustCrypto key), run it through the conversion path, and assert the output is exactly the fixed width and round-trips back to the same `r`/`s`.
- [x] Add a negative-space test that a malformed DER signature yields `KeyError`, and confirm an RS\*/PS\* signature passes through byte-identical.

## Definition of done

- [x] For ES256/384/512, `sign`'s post-KMS conversion yields raw `r || s` of exactly 64/96/132 bytes; RS\*/PS\* output is byte-identical to the KMS response.
- [x] Negative-space test: a malformed DER ECDSA signature returns `KeyError`, not a panic or a truncated buffer.
- [x] The three curve widths are named constants; the `sign` function carries at least two meaningful assertions and the ES\* branch is exhaustive over the accepted algorithms.
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: run `cargo nextest run -p oidc-exchange-adapters kms::tests` and observe the per-curve DER→raw conversion tests asserting the fixed-width output pass.

## Open questions

- None.
