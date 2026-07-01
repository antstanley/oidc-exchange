# Done Certificate — Task 02: ES512 JWK arm and p521 dependency

**Task:** [02-es512_jwk_and_p521_dep.md](02-es512_jwk_and_p521_dep.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified

> This certificate is a verification protocol for Task 02. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 02) ≡ every obligation O1…O5 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** `parse_spki_to_jwk` publishes a P-521 JWK for ES512 (`crv: "P-521"`, 66-byte coordinates), and the `p521` crate is available for the P-521 curve type used here and in tasks 03/04.
- **P2 — Obligations.** The task is done iff O1…O5 all hold; O5 is the Reviewable item.
- **P3 — Invariants.** Must not break the ES256/ES384 arms of `parse_spki_to_jwk` nor the RSA arm; must not change existing JWK output for those algorithms.

## Obligations

- **O1 — P-521 key produces a P-521 JWK; ES256/384 unchanged.**
  - *Claim:* `parse_spki_to_jwk(spki, "ES512", kid)` yields `crv == "P-521"` with 66-byte `x`/`y` (~88 base64url chars), and ES256/ES384 output is unchanged.
  - *Evidence to collect:* read the EC arm of `crates/adapters/src/kms/mod.rs` (was `:101-141`) and confirm `"ES512"` maps to `("P-521", 66)`. Run the new P-521 variant of `test_parse_ec_public_key_to_jwk` and the existing ES256 test in `kms::tests` — expect both PASS.
  - *Checks:* resolve the curve-parameter match — confirm the `_ => unreachable!()` was replaced by a real `ES512` arm and the match is exhaustive over `ES256|ES384|ES512`.
  - *Status:* ☐ unverified

- **O2 — Negative-space test: short or non-`0x04` SPKI returns KeyError.**
  - *Claim:* an SPKI shorter than a P-521 point, or lacking the `0x04` uncompressed-point prefix, returns `Err(KeyError)` rather than panicking.
  - *Evidence to collect:* run the negative test feeding a too-short / bad-prefix buffer through the ES512 path — expect `KeyError`. Trace the length check and prefix check to confirm they cover the 66-byte coordinate case.
  - *Status:* ☐ unverified

- **O3 — Coordinate length is a named constant/table; ≥2 assertions; match exhaustive.**
  - *Claim:* `66` is a named constant or match-table entry, the EC arm carries at least two meaningful assertions, and no `unreachable!` is reachable for an accepted algorithm.
  - *Evidence to collect:* read the EC arm and grep for the literal `66`; confirm it is bound to a name or table. Count assertions in the P-521 test.
  - *Status:* ☐ unverified

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests, lint, and format are clean with `p521` added.
  - *Evidence to collect:* confirm `p521` is in `crates/adapters/Cargo.toml` on the `0.14.0-rc` line with the `ecdsa` feature and resolves in `Cargo.lock`. Run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` (per `.specs/development-guidelines.md` §Definition of done) — expect all clean.
  - *Status:* ☐ unverified

- **O5 — Reviewable: new P-521 parse test passes.**
  - *Claim:* a reviewer runs the kms tests and sees the P-521 `parse_spki_to_jwk` test pass.
  - *Evidence to collect:* run `cargo nextest run -p oidc-exchange-adapters kms::tests` — expect the P-521 test to PASS with `crv == "P-521"`.
  - *Status:* ☐ unverified

## Regression check

- `KmsKeyManager::fetch_public_jwk` for an ES256 key → still yields `crv == "P-256"` with 32-byte coordinates : ☐ (PRESERVED / REGRESSION)
- The RSA arm and unsupported-algorithm arm of `parse_spki_to_jwk` are untouched → their tests still pass : ☐ (PRESERVED / REGRESSION)

## Residue

- Outside the DoD: the `p521` dependency is consumed for JWK output here; its use in the `sign` conversion and local `verify` is scoped to tasks 03 and 04.

## Conclusion

VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
