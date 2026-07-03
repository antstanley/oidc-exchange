# Change: Fix KMS ECDSA signature encoding and JWK output

**Status:** Merged · **Date:** 2026-07-01 · **Merged:** 2026-07-03 · **Owner:** Ant Stanley · **Target:** crates/adapters

Make the KMS key manager produce JWS-valid ES\* signatures (convert KMS's DER-encoded ECDSA
output to raw `r || s`), emit RFC 7518-compliant RSA JWKs (no leading zero octets in `n`/`e`),
and publish a JWK for ES512, which the adapter already signs with but cannot describe at
`/keys`. `verify` moves off KMS Verify entirely: signatures are checked locally against the
cached public key, removing a KMS round-trip from every access-token revoke.

---

## Motivation

The KMS adapter returns the AWS KMS `Sign` response verbatim, but KMS encodes ECDSA signatures
as DER (ASN.1 `Ecdsa-Sig-Value`) while JWS ES256/384/512 requires the raw fixed-length
`r || s` form (64/96/132 bytes). Every ES\*-on-KMS deployment therefore mints access tokens
that no standard JWT library can verify against the served JWKS. The bug is invisible
internally because the adapter's own `verify` path round-trips through KMS `Verify`, which
accepts the same DER form.

Two JWK defects compound this. RSA `n`/`e` are encoded from `to_be_bytes()` including leading
zero octets (`e = 65537` encodes as `AAAAAAABAAE` instead of `AQAB`), violating RFC 7518's
Base64urlUInt rule, so strict consumers (WebCrypto `importKey`, RFC 7638 thumbprints) reject
or mis-derive the key. And `signing_algorithm()` accepts ES512 while `parse_spki_to_jwk` only
handles ES256/ES384, so an ES512 deployment signs with a key that never appears at `/keys` —
despite the canonical adapter inventory claiming "RS/PS/ES 256/384/512". RSA and PSS
signatures are unaffected.

---

## Affected spec pages

| Canonical page                                                                               | Nature of change                                                                                                         |
| -------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| [`.specs/service/specs/02-ports-and-adapters.md`](../service/specs/02-ports-and-adapters.md) | Document the JWS wire form of `sign` for ES\* on KMS, local `verify` against the cached public key, and RFC 7518-compliant JWK output covering P-256/384/521 |

---

## Proposed changes

### `.specs/service/specs/02-ports-and-adapters.md` → Port traits → KeyManager (Modify)

> `sign` returns signature bytes in the form the JWS serialization uses directly. For the ES\*
> algorithms the KMS adapter converts the DER-encoded `Ecdsa-Sig-Value` returned by KMS Sign
> into raw fixed-length `r || s` (64/96/132 bytes for ES256/384/512). RSA and PSS signatures
> are already in JWS form and pass through unchanged. `verify` does not call KMS: it checks
> the signature locally against the cached public key (the same SPKI fetched once for the
> JWK), so revoking an access token costs no KMS round-trip. Local verification consumes the
> raw `r || s` form directly, so no raw→DER conversion exists anywhere in the adapter.

### `.specs/service/specs/02-ports-and-adapters.md` → Adapter inventory (Modify)

> | KeyManager | AWS KMS | `adapters/kms` | RS/PS/ES 256/384/512; ECDSA DER→raw JWS conversion on sign; local verify against the cached public key; JWK cached on `OnceCell`; `Sign`/`GetPublicKey` |
>
> The KMS adapter's JWKs are strict RFC 7517/7518: RSA `n`/`e` are Base64urlUInt with no
> leading zero octets (`e = 65537` encodes as `AQAB`), and EC keys cover P-256, P-384, and
> P-521, so every algorithm the adapter signs with has a published JWK at `/keys`.

---

## Type changes

None. No domain entity or config field changes; the `KeyManager` trait signature is unchanged.

---

## Implementation notes

1. `crates/adapters/src/kms/mod.rs:150-175` — in `sign`, when `self.algorithm` starts with
   `ES`, convert the KMS signature from DER to raw `r || s` via the `ecdsa` crate
   (`ecdsa::Signature::from_der(..)?.to_bytes()`, which yields the fixed-width form —
   64/96/132 bytes for P-256/384/521); no hand-rolled ASN.1.
2. `crates/adapters/src/kms/mod.rs:177-195` — rewrite `verify` to validate locally instead of
   calling KMS Verify: parse the cached SPKI into a verifying key and check the signature
   in-process. RS\*/PS\* use the `rsa` crate (`pkcs1v15`/`pss`) with `sha2`; ES256/ES384 use
   `p256`/`p384` `ecdsa::VerifyingKey` — all already in the dependency tree
   (`crates/adapters/Cargo.toml:19-21`, `ecdsa`/`signature` via Cargo.lock). ES512 needs the
   RustCrypto `p521` crate (same `0.14.0-rc` line as `p256`/`p384`, `ecdsa` feature) added to
   `crates/adapters/Cargo.toml`. Local verification consumes raw `r || s` directly
   (`ecdsa::Signature::from_slice`), so no raw→DER conversion is needed.
3. `crates/adapters/src/kms/mod.rs:18` — the `OnceCell` caches only the JWK JSON; cache the
   SPKI DER from the same `GetPublicKey` fetch (`fetch_public_jwk`, `:56-75`) so `verify` has
   key material without re-fetching (e.g. store `(spki_der, jwk)` in the cell, or a second
   `OnceCell` filled by the shared fetch).
4. `crates/adapters/src/kms/mod.rs:89-90` — strip leading zero octets from the `n`/`e`
   big-endian byte strings before base64url encoding.
5. `crates/adapters/src/kms/mod.rs:101-141` — add an `ES512` arm to `parse_spki_to_jwk`
   (`crv: "P-521"`, `coord_len: 66`); update the doc comment at `:78-80`.
6. Tests (`kms/mod.rs:214` onward): DER→raw conversion vectors per curve, local `verify`
   accepting a locally signed raw `r || s` and rejecting a tampered one, `e == "AQAB"` for a
   generated RSA key, and a P-521 variant of `test_parse_ec_public_key_to_jwk`.

References: RFC 7515 §3 / RFC 7518 §3.4 (ECDSA raw form), RFC 7518 §6.3 (Base64urlUInt),
AWS KMS `Sign` API docs (ECDSA output is DER).

---

## Merge plan

1. Apply both `Proposed changes` blocks to
   [02-ports-and-adapters.md](../service/specs/02-ports-and-adapters.md); bump its `**Date:**`.
2. No schema change.
3. Flip `**Status:**` to `Merged`, stamp `**Merged:**`, move to `.specs/changes/merged/`.
4. Update `.specs/README.md`.

---

## Assumptions and open questions

### Assumptions

- KMS returns ECDSA signatures DER-encoded for all `ECDSA_SHA_*` signing algorithms (per AWS
  documentation), so the conversion is unconditional for ES\*.
- No deployment depends on the current (broken) DER output; the fix is not treated as a
  breaking change.

### Decisions

- _Convert in the adapter, not the core._ **The JWS wire form is part of the `KeyManager`
  contract; each adapter is responsible for meeting it.** The local Ed25519 adapter already
  returns JWS-ready bytes.
- _Verify locally against the cached public key._ **`verify` validates signatures in-process
  using the SPKI already fetched for the JWK, not KMS Verify.** This removes a KMS network
  round-trip from every access-token revoke and makes raw→DER conversion unnecessary, since
  local verification consumes the raw `r || s` form directly.
- _Reuse the `ecdsa` crate for DER→raw._ **`ecdsa::Signature::from_der(..).to_bytes()` does
  the conversion; no hand-rolled ASN.1.** The generic `ecdsa` crate is already in the tree via
  `p256`/`p384`; ES512 additionally pulls in the RustCrypto `p521` crate from the same release
  line.

### Open questions

- (None at this stage.)
