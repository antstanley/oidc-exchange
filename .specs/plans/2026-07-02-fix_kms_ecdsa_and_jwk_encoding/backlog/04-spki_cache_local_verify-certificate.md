# Done Certificate — Task 04: SPKI cache and local verify

**Task:** [04-spki_cache_local_verify.md](04-spki_cache_local_verify.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified

> This certificate is a verification protocol for Task 04. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 04) ≡ every obligation O1…O5 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** `verify` validates signatures in-process against the SPKI cached from the single `GetPublicKey`, consuming raw `r || s` directly, with no KMS Verify round-trip and no raw→DER conversion.
- **P2 — Obligations.** The task is done iff O1…O5 all hold; O5 is the Reviewable item.
- **P3 — Invariants.** Must not break `public_jwk` (still returns the JWK), nor the single-fetch caching contract; the `KeyManager::verify` trait signature is unchanged.

## Obligations

- **O1 — Local verify accepts valid, rejects tampered, no KMS Verify.**
  - *Claim:* for every supported algorithm, `verify` returns `true` for a correctly signed payload and `false` for a tampered signature or payload, without calling KMS Verify.
  - *Evidence to collect:* read `crates/adapters/src/kms/mod.rs` `verify` (was `:177-195`) and confirm the KMS `.verify()` call is gone, replaced by `rsa` (`pkcs1v15`/`pss` + `sha2`) and `ecdsa::VerifyingKey` (`p256`/`p384`/`p521`) checks. Run the accept-valid and reject-tampered tests in `kms::tests` — expect PASS.
  - *Checks:* resolve the signature parse in the ES\* path — confirm it is `ecdsa::Signature::from_slice` on raw `r || s`, not `from_der`; confirm no KMS client `.verify()` remains anywhere in the module.
  - *Status:* ☐ unverified

- **O2 — Negative-space test: unparseable key/unsupported algorithm → KeyError; tampered → Ok(false).**
  - *Claim:* unparseable SPKI or an unsupported algorithm returns `Err(KeyError)`, while a tampered but well-formed signature returns `Ok(false)` (no error, no panic).
  - *Evidence to collect:* run the negative tests — expect `KeyError` for bad key material and `Ok(false)` for the tampered-signature case. Trace the error/`false` split and confirm the algorithm match stays exhaustive.
  - *Status:* ☐ unverified

- **O3 — SPKI fetched once and cached; ≥2 assertions; no raw→DER.**
  - *Claim:* the SPKI DER is obtained from the same `GetPublicKey` as the JWK and cached (`OnceCell`), `verify` and the caching path each carry ≥2 meaningful assertions, and no raw→DER conversion exists in the adapter.
  - *Evidence to collect:* read the cache setup (was `:18`, `:56-75`) and confirm one shared fetch fills both JWK and SPKI. Grep the module for any `to_der`/raw→DER call — expect none.
  - *Checks:* resolve the cached-SPKI accessor used by `verify` — confirm it reads the cell filled by `fetch_public_jwk`, not a second `GetPublicKey` call.
  - *Status:* ☐ unverified

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests, lint, and format are clean.
  - *Evidence to collect:* run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` (per `.specs/development-guidelines.md` §Definition of done) — expect all clean.
  - *Status:* ☐ unverified

- **O5 — Reviewable: local-verify accept/reject tests pass with no KMS interaction.**
  - *Claim:* a reviewer runs the kms tests and sees the accept-valid and reject-tampered local-verify tests pass without any KMS client call.
  - *Evidence to collect:* run `cargo nextest run -p oidc-exchange-adapters kms::tests` — expect the local-verify tests to PASS; confirm by reading the tests that they construct no KMS Verify expectation.
  - *Status:* ☐ unverified

## Regression check

- `public_jwk()` callers (e.g. the `/keys` handler and `test_kms_sign_integration`) → still receive the JWK unchanged after the cache holds `(spki_der, jwk)` : ☐ (PRESERVED / REGRESSION)
- The revoke flow that calls `KeyManager::verify` on an access-token JWT → still authenticates a valid token and rejects an invalid one, now locally : ☐ (PRESERVED / REGRESSION)

## Residue

- Outside the DoD: whether the revoke flow's best-effort semantics change with local verification is a behavioural note for the server crate, not an obligation of this adapter task.

## Conclusion

VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
