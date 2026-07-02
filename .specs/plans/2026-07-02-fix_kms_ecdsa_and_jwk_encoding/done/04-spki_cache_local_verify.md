# Task 04 — SPKI cache and local verify

**Plan:** [plan.md](../plan.md) · **Certificate:** [04-spki_cache_local_verify-certificate.md](04-spki_cache_local_verify-certificate.md)

**Implements:** [.specs/service/specs/02-ports-and-adapters.md](../../../service/specs/02-ports-and-adapters.md) §Port traits → KeyManager (`verify` does not call KMS; it checks the signature locally against the cached public key, consuming raw `r || s` directly); change spec Implementation notes 2 and 3.
**Depends on:** 02 (build — the P-521 curve type for ES512 local verify), 03 (review — `verify` consumes the raw `r || s` wire form `sign` now produces; sequencing 03 first makes the adapter sign→verify round-trip the reviewable state).
**Produces:** a `verify` that validates signatures in-process against the SPKI already fetched for the JWK — RS\*/PS\* via the `rsa` crate (`pkcs1v15`/`pss` + `sha2`), ES256/384/512 via `p256`/`p384`/`p521` `ecdsa::VerifyingKey` consuming raw `r || s`. Revoking an access token no longer costs a KMS Verify round-trip, and no raw→DER conversion exists anywhere in the adapter.
**Pointers:** `crates/adapters/src/kms/mod.rs:18` (the JWK-only `OnceCell`); `crates/adapters/src/kms/mod.rs:56-75` (`fetch_public_jwk`, the single `GetPublicKey`); `crates/adapters/src/kms/mod.rs:177-195` (`verify`, currently KMS Verify).

## Steps

- [x] Cache the SPKI DER from the same `GetPublicKey` fetch that builds the JWK — store `(spki_der, jwk)` in the existing cell or add a second `OnceCell` filled by the shared fetch — so `verify` gets key material without a second KMS call; keep `public_jwk` returning only the JWK.
- [x] Rewrite `verify` to parse the cached SPKI into a verifying key and check the signature locally: RS\*/PS\* with `rsa` `pkcs1v15`/`pss` and the matching `sha2` digest; ES256/384/512 with the curve's `ecdsa::VerifyingKey` and `ecdsa::Signature::from_slice` on the raw `r || s`.
- [x] Remove the KMS Verify call and its client round-trip entirely; ensure no raw→DER conversion is introduced (local verification consumes raw `r || s`).
- [x] Map every parse/verify failure to a domain result: a signature that does not validate returns `Ok(false)`, unparseable key material or an unsupported algorithm returns `Err(KeyError)`; keep the algorithm match exhaustive.
- [x] Add tests: for each algorithm family, sign a payload with a locally-generated key, build the SPKI, and assert `verify` returns `true`; then tamper one byte of the signature (and separately the payload) and assert `verify` returns `false`.

## Definition of done

- [x] `verify` validates a locally-signed raw `r || s` (or RS\*/PS\*) signature as `true` and a tampered signature or payload as `false`, for every supported algorithm, without calling KMS Verify.
- [x] Negative-space test: unparseable SPKI or an unsupported algorithm returns `KeyError`, and a tampered signature returns `Ok(false)` (not an error and not a panic).
- [x] The SPKI is fetched once (shared with the JWK fetch) and cached; `verify` and the caching path each carry at least two meaningful assertions; no raw→DER conversion exists in the adapter.
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: run `cargo nextest run -p oidc-exchange-adapters kms::tests` and observe the local-verify accept-valid and reject-tampered tests pass with no KMS client interaction.

## Open questions

- None.
