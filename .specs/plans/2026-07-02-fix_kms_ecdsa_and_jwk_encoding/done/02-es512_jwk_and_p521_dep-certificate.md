# Done Certificate — Task 02: ES512 JWK arm and p521 dependency

**Task:** [02-es512_jwk_and_p521_dep.md](02-es512_jwk_and_p521_dep.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

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
  - *Status:* ☑ SATISFIED — EC arm at `crates/adapters/src/kms/mod.rs:129-136` maps `"ES512" => ("P-521", EC_COORD_LEN_P521)` (= 66). `kms::tests::test_parse_p521_public_key_to_jwk` PASS (`crv == "P-521"`, `x`/`y` exactly 88 base64url chars); existing `test_parse_ec_public_key_to_jwk` PASS (`crv == "P-256"`, ~43-char coords, output unchanged). The inner match has real arms for all three accepted algorithms; the remaining `_ => unreachable!(...)` is only the Rust catch-all required for `&str` matching, unreachable because the outer arm restricts to `"ES256" | "ES384" | "ES512"`.

- **O2 — Negative-space test: short or non-`0x04` SPKI returns KeyError.**
  - *Claim:* an SPKI shorter than a P-521 point, or lacking the `0x04` uncompressed-point prefix, returns `Err(KeyError)` rather than panicking.
  - *Evidence to collect:* run the negative test feeding a too-short / bad-prefix buffer through the ES512 path — expect `KeyError`. Trace the length check and prefix check to confirm they cover the 66-byte coordinate case.
  - *Status:* ☑ SATISFIED — committed tests `test_parse_ec_public_key_spki_too_short_is_key_error` and `test_parse_ec_public_key_missing_uncompressed_prefix_is_key_error` PASS (they parametrize on ES256, exercising the shared checks). The checks are shared code binding `coord_len` before use: length check at `mod.rs:138-146` (`point_len = 1 + 2*66 = 133` for ES512), prefix check at `mod.rs:148-156`. Validator additionally ran a temporary test feeding a 132-byte buffer and a 133-byte `0x02`-prefixed buffer through `parse_spki_to_jwk(.., "ES512", ..)` — both returned `Err(Error::KeyError)`, PASS (test removed after evidence collection). No panic on either path.

- **O3 — Coordinate length is a named constant/table; ≥2 assertions; match exhaustive.**
  - *Claim:* `66` is a named constant or match-table entry, the EC arm carries at least two meaningful assertions, and no `unreachable!` is reachable for an accepted algorithm.
  - *Evidence to collect:* read the EC arm and grep for the literal `66`; confirm it is bound to a name or table. Count assertions in the P-521 test.
  - *Status:* ☑ SATISFIED — `66` is bound to the named constant `EC_COORD_LEN_P521` at `mod.rs:91` (with doc comment deriving 66 = ceil(521/8)); `grep -n 66` shows the literal appears only in that constant's definition and in test comments/assert messages. Sibling constants `EC_COORD_LEN_P256`/`EC_COORD_LEN_P384` replace the former bare `32`/`48`. The P-521 test carries 6 assertions (`kty`, `crv`, `alg`, `kid`, `x_len == 88`, `y_len == 88`). Match exhaustiveness confirmed under O1.

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests, lint, and format are clean with `p521` added.
  - *Evidence to collect:* confirm `p521` is in `crates/adapters/Cargo.toml` on the `0.14.0-rc` line with the `ecdsa` feature and resolves in `Cargo.lock`. Run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` (per `.specs/development-guidelines.md` §Definition of done) — expect all clean.
  - *Status:* ☑ SATISFIED — `crates/adapters/Cargo.toml:22` declares `p521 = { version = "0.14.0-rc.1", features = ["ecdsa"] }`; `Cargo.lock` resolves it to `p521 0.14.0-rc.15` (same rc line as `p256`/`p384` 0.14.0-rc.15). `cargo fmt --check` exit 0; `cargo clippy --workspace -- -D warnings` clean; `cargo nextest run --workspace` → 195 passed, 2 skipped, 0 failed.

- **O5 — Reviewable: new P-521 parse test passes.**
  - *Claim:* a reviewer runs the kms tests and sees the P-521 `parse_spki_to_jwk` test pass.
  - *Evidence to collect:* run `cargo nextest run -p oidc-exchange-adapters kms::tests` — expect the P-521 test to PASS with `crv == "P-521"`.
  - *Status:* ☑ SATISFIED — `cargo nextest run -p oidc-exchange-adapters kms::tests` → 9 tests run, 9 passed, including `test_parse_p521_public_key_to_jwk` (asserts `crv == "P-521"` and 88-char coordinates).

## Regression check

- `KmsKeyManager::fetch_public_jwk` for an ES256 key → still yields `crv == "P-256"` with 32-byte coordinates : ☑ PRESERVED — `fetch_public_jwk` is untouched and delegates to `parse_spki_to_jwk`; the ES256 arm's only change is `32` → `EC_COORD_LEN_P256` (= 32, semantically identical); `test_parse_ec_public_key_to_jwk` PASS with `crv == "P-256"` and ~43-char (32-byte) coordinates.
- The RSA arm and unsupported-algorithm arm of `parse_spki_to_jwk` are untouched → their tests still pass : ☑ PRESERVED — the diff shows no edits to either arm; `test_parse_rsa_public_key_to_jwk` and `test_parse_spki_unsupported_algorithm` both PASS.

## Residue

- Outside the DoD: the `p521` dependency is consumed for JWK output here; its use in the `sign` conversion and local `verify` is scoped to tasks 03 and 04.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O5 all SATISFIED with collected evidence (P-521 JWK test passes with `crv == "P-521"` and 66-byte coordinates; ES512 negative paths return `KeyError` — verified both via the committed ES256-parametrized tests over the shared checks and a validator-run ES512-path probe; `66` is the named constant `EC_COORD_LEN_P521`; fmt/clippy/workspace tests all clean with `p521 0.14.0-rc.15` resolved), and both regression callers are PRESERVED. Minor note: the committed negative-space tests parametrize on ES256 rather than ES512; the checks are shared code and the ES512 path was traced and probed, but an ES512-parametrized negative test would make the evidence self-contained.
