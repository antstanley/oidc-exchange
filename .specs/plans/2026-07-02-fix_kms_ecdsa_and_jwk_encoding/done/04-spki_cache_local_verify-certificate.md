# Done Certificate — Task 04: SPKI cache and local verify

**Task:** [04-spki_cache_local_verify.md](04-spki_cache_local_verify.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

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
  - *Status:* ✅ SATISFIED — `verify` (`crates/adapters/src/kms/mod.rs:430-441`) reads the cached `(spki_der, jwk)` cell and dispatches to `verify_locally` (`:364`); the KMS `.verify()` round-trip is gone (grep finds no KMS client verify — the only `.verify(...)` calls resolve to `rsa::signature::Verifier` / `ecdsa::VerifyingKey`, imported `Verifier as _`). ES\* parses via `p256/p384/p521::ecdsa::Signature::from_slice` at `:319/:335/:351` (fully qualified, no shadowing, not `from_der`). All 9 per-algorithm accept-valid / reject-tampered-signature / reject-tampered-payload tests PASS (`test_verify_locally_{rs256,rs384,rs512,ps256,ps384,ps512,es256,es384,es512}_accepts_valid_and_rejects_tampering`).

- **O2 — Negative-space test: unparseable key/unsupported algorithm → KeyError; tampered → Ok(false).**
  - *Claim:* unparseable SPKI or an unsupported algorithm returns `Err(KeyError)`, while a tampered but well-formed signature returns `Ok(false)` (no error, no panic).
  - *Evidence to collect:* run the negative tests — expect `KeyError` for bad key material and `Ok(false)` for the tampered-signature case. Trace the error/`false` split and confirm the algorithm match stays exhaustive.
  - *Status:* ✅ SATISFIED — `test_verify_locally_unsupported_algorithm_is_key_error` and `test_verify_locally_unparseable_spki_is_key_error` PASS, both asserting `Err(Error::KeyError { .. })`; `test_verify_locally_malformed_signature_bytes_returns_false_not_error` PASS (`Ok(false)`, no panic), and every per-algorithm test asserts tampered signature/payload → `Ok(false)`. Trace: in each `verify_*` helper an unparseable SPKI maps to `Err(KeyError)` while signature parse/verify failure short-circuits to `Ok(false)`; the `verify_locally` match covers all 9 algorithms with a final `other => Err(KeyError)` arm — exhaustive.

- **O3 — SPKI fetched once and cached; ≥2 assertions; no raw→DER.**
  - *Claim:* the SPKI DER is obtained from the same `GetPublicKey` as the JWK and cached (`OnceCell`), `verify` and the caching path each carry ≥2 meaningful assertions, and no raw→DER conversion exists in the adapter.
  - *Evidence to collect:* read the cache setup (was `:18`, `:56-75`) and confirm one shared fetch fills both JWK and SPKI. Grep the module for any `to_der`/raw→DER call — expect none.
  - *Checks:* resolve the cached-SPKI accessor used by `verify` — confirm it reads the cell filled by `fetch_public_jwk`, not a second `GetPublicKey` call.
  - *Status:* ✅ SATISFIED — `fetch_public_key_material` (`crates/adapters/src/kms/mod.rs:56-85`, renamed from `fetch_public_jwk`) makes the single `GetPublicKey` call and returns `(spki_der, jwk)` into `OnceCell<(Vec<u8>, serde_json::Value)>` (`:22`); both `verify` (`:437`) and `public_jwk` (`:445`) resolve through `get_or_try_init` on that same cell — no second `GetPublicKey`. `test_public_key_cache_fetches_shared_material_only_once` PASS with 2 assertions (fetch count == 1; identical SPKI bytes); each per-algorithm verify test carries 3 assertions. Grep for raw→DER: only `der_to_raw_ecdsa` (DER→raw, sign path) exists; verify consumes raw `r || s` via `from_slice` — no raw→DER conversion anywhere in the adapter.

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests, lint, and format are clean.
  - *Evidence to collect:* run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` (per `.specs/development-guidelines.md` §Definition of done) — expect all clean.
  - *Status:* ✅ SATISFIED — `cargo fmt --check` exit 0; `cargo clippy --workspace --all-targets -- -D warnings` finished with no warnings; `cargo nextest run --workspace` → 215 tests run, 215 passed, 2 skipped (ignored integration tests requiring LocalStack/KMS).

- **O5 — Reviewable: local-verify accept/reject tests pass with no KMS interaction.**
  - *Claim:* a reviewer runs the kms tests and sees the accept-valid and reject-tampered local-verify tests pass without any KMS client call.
  - *Evidence to collect:* run `cargo nextest run -p oidc-exchange-adapters kms::tests` — expect the local-verify tests to PASS; confirm by reading the tests that they construct no KMS Verify expectation.
  - *Status:* ✅ SATISFIED — ran `cargo nextest run -p oidc-exchange-adapters kms::tests` → 29 tests run, 29 passed (45 skipped, other modules). The local-verify tests exercise the pure `verify_locally` function over locally generated keys (`rsa::RsaPrivateKey::new`, `SigningKey::generate`) and locally built SPKI DER — no KMS client, mock, or Verify expectation is constructed anywhere in them.

## Regression check

- `public_jwk()` callers (e.g. the `/keys` handler and `test_kms_sign_integration`) → still receive the JWK unchanged after the cache holds `(spki_der, jwk)` : ✅ PRESERVED — `crates/server/src/routes/keys.rs:10` receives the JWK via `public_jwk`, which now returns `jwk.clone()` from the tuple; the JWK is built by the same `parse_spki_to_jwk` with the same inputs, so its content is byte-identical. `test_kms_sign_integration` is `#[ignore]` (needs LocalStack) and compiles unchanged.
- The revoke flow that calls `KeyManager::verify` on an access-token JWT → still authenticates a valid token and rejects an invalid one, now locally : ✅ PRESERVED — `crates/core/src/service/revoke.rs:52-56` (`verify_and_extract_sub`) calls the unchanged trait signature `verify(&self, payload, signature) -> Result<bool>`; a valid raw `r || s` JWS signature (the form `sign` now emits per task 03) verifies locally → `Ok(true)`, an invalid one → `Ok(false)` → `None`, same semantics with the KMS round-trip removed.

## Residue

- Outside the DoD: whether the revoke flow's best-effort semantics change with local verification is a behavioural note for the server crate, not an obligation of this adapter task.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O5 all SATISFIED with direct evidence — local verify replaces KMS Verify for all 9 algorithms (29/29 kms tests, 215/215 workspace tests, fmt/clippy clean), the SPKI is cached from the single shared `GetPublicKey`, no raw→DER conversion exists, and both named downstream callers (`/keys` JWK handler, revoke-flow `verify`) are PRESERVED.
