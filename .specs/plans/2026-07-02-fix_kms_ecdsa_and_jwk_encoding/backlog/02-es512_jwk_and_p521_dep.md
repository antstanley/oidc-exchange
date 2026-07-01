# Task 02 — ES512 JWK arm and p521 dependency

**Plan:** [plan.md](../plan.md) · **Certificate:** [02-es512_jwk_and_p521_dep-certificate.md](02-es512_jwk_and_p521_dep-certificate.md)

**Implements:** [.specs/service/specs/02-ports-and-adapters.md](../../../service/specs/02-ports-and-adapters.md) §Adapter inventory (EC keys cover P-256, P-384, and P-521, so every algorithm the adapter signs with has a published JWK); change spec Implementation note 5 (and the `p521` dependency from note 2).
**Depends on:** —
**Produces:** a P-521 JWK for ES512 — `parse_spki_to_jwk` gains an `ES512` arm (`crv: "P-521"`, coordinate length 66), so an ES512 deployment's signing key finally appears at `/keys`. Adds the `p521` crate (same `0.14.0-rc` line as `p256`/`p384`, `ecdsa` feature) that this task and tasks 03/04 need for the P-521 curve type.
**Pointers:** `crates/adapters/src/kms/mod.rs:101-141` (the EC branch of `parse_spki_to_jwk`, currently `"ES256" | "ES384"`); doc comment `crates/adapters/src/kms/mod.rs:78-80`; `crates/adapters/Cargo.toml:20-21` (where `p256`/`p384` are declared); existing EC test `crates/adapters/src/kms/mod.rs:291-321`.

## Steps

- [ ] Add `p521` to `crates/adapters/Cargo.toml` on the same `0.14.0-rc` release line as `p256`/`p384` with the `ecdsa` feature; confirm it resolves in `Cargo.lock`.
- [ ] Extend the EC match arm in `parse_spki_to_jwk` to accept `"ES512"` alongside `"ES256"`/`"ES384"`, mapping it to `("P-521", 66)`; keep the existing uncompressed-point length check and `0x04`-prefix validation covering the 66-byte coordinate.
- [ ] Replace the `_ => unreachable!()` in the curve-parameter match so `ES512` is a real arm and the match stays exhaustive over the accepted algorithms.
- [ ] Update the `parse_spki_to_jwk` doc comment at `:78-80` to state EC support covers ES256/ES384/ES512 (P-256/P-384/P-521).
- [ ] Add a P-521 variant of `test_parse_ec_public_key_to_jwk` (generate a P-521 key, encode SPKI, assert `crv == "P-521"` and `x`/`y` are ~88 base64url chars for 66-byte coordinates).

## Definition of done

- [ ] A generated P-521 key produces a JWK with `crv == "P-521"` and 66-byte `x`/`y` coordinates; ES256/ES384 JWK output is unchanged.
- [ ] Negative-space test: an SPKI too short for a P-521 point, or one without the `0x04` uncompressed-point prefix, returns a `KeyError` rather than panicking.
- [ ] The coordinate length (66) is a named constant or table entry, not a bare literal; the touched arm carries at least two meaningful assertions and the curve match is exhaustive (no `unreachable!` reachable for an accepted algorithm).
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: run `cargo nextest run -p oidc-exchange-adapters kms::tests` and observe the new P-521 `parse_spki_to_jwk` test pass.

## Open questions

- None.
