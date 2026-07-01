# Task 01 — RSA JWK Base64urlUInt encoding

**Plan:** [plan.md](../plan.md) · **Certificate:** [01-rsa_jwk_base64urluint-certificate.md](01-rsa_jwk_base64urluint-certificate.md)

**Implements:** [.specs/service/specs/02-ports-and-adapters.md](../../../service/specs/02-ports-and-adapters.md) §Adapter inventory (RSA `n`/`e` are Base64urlUInt with no leading zero octets); change spec Implementation note 4.
**Depends on:** —
**Produces:** RSA JWKs at `/keys` whose `n`/`e` follow RFC 7518 §6.3 Base64urlUInt — no leading zero octets, so `e = 65537` encodes as `AQAB` and strict consumers (WebCrypto `importKey`, RFC 7638 thumbprints) accept the key.
**Pointers:** `crates/adapters/src/kms/mod.rs:89-90` (the `n`/`e` `to_be_bytes()` encoding in `parse_spki_to_jwk`); existing RSA test `crates/adapters/src/kms/mod.rs:324-342`.

## Steps

- [ ] In `parse_spki_to_jwk`, strip leading `0x00` octets from the big-endian `n` and `e` byte strings before base64url-encoding (a minimal Base64urlUInt encoder that removes leading zeros but preserves a single zero byte for a zero value).
- [ ] Factor the strip-and-encode into a small helper so `n` and `e` share one code path and it is unit-testable in isolation.
- [ ] Extend `test_parse_rsa_public_key_to_jwk` (or add a sibling test) to assert `jwk["e"] == "AQAB"` for a generated RSA key and that neither `n` nor `e` begins with the base64url encoding of a `0x00` octet.
- [ ] Add a negative-space assertion that a value with a genuine leading zero (e.g. an exponent whose top byte is zero) still encodes without the zero, and that the helper never emits an empty string.

## Definition of done

- [ ] A generated 2048-bit RSA key produces a JWK with `e == "AQAB"` and no `n`/`e` leading zero octets.
- [ ] Negative-space test: the encoder strips leading zeros but preserves a single byte for a zero-valued input (never emits an empty string).
- [ ] The touched `parse_spki_to_jwk` RSA arm and the new helper each carry at least two meaningful assertions; any magic width is a named constant.
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: run `cargo nextest run -p oidc-exchange-adapters kms::tests` and observe the RSA JWK test asserting `e == "AQAB"` pass.

## Open questions

- None.
