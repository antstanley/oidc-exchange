# Task 05 — Sync canonical spec page

**Plan:** [plan.md](../plan.md) · **Certificate:** [05-sync_spec_page-certificate.md](05-sync_spec_page-certificate.md)

**Implements:** [.specs/service/specs/02-ports-and-adapters.md](../../../service/specs/02-ports-and-adapters.md) §Port traits → KeyManager and §Adapter inventory; change spec Proposed changes blocks and Merge plan steps 1–2.
**Depends on:** 01, 03, 04 (review — the page documents behaviour only demonstrable once the RSA JWK fix, the DER→raw `sign`, and the local `verify` have landed).
**Produces:** the canonical page 02-ports-and-adapters.md describes the shipped adapter: `sign`'s JWS wire form (ES\* DER→raw), local `verify` against the cached public key, RFC 7518-compliant JWK output covering P-256/384/521, and the updated KMS row of the Adapter inventory.
**Pointers:** `.specs/service/specs/02-ports-and-adapters.md:42-54` (KeyManager trait section and its `verify` note); `.specs/service/specs/02-ports-and-adapters.md:87` (KMS row of the Adapter inventory); `.specs/service/specs/02-ports-and-adapters.md:3` (`**Date:**` line to bump).

## Steps

- [ ] Add the KeyManager `sign`/`verify` wire-form paragraph from the change spec's first Proposed changes block to the KeyManager section (or adjacent note): ES\* DER→raw on `sign`, RS/PS pass-through, local `verify` against the cached SPKI consuming raw `r || s`, no raw→DER conversion anywhere.
- [ ] Replace the KMS row of the Adapter inventory with the change spec's version (`RS/PS/ES 256/384/512; ECDSA DER→raw JWS conversion on sign; local verify against the cached public key; JWK cached on OnceCell; Sign/GetPublicKey`) — note the row no longer lists KMS `Verify`.
- [ ] Add the strict-RFC-7517/7518 note (RSA `n`/`e` Base64urlUInt with no leading zero octets, `e = 65537` → `AQAB`; EC keys cover P-256/P-384/P-521) after the inventory table.
- [ ] Bump the page's `**Date:**` to 2026-07-02; leave its `**Status:**` and the `.specs/README.md` index and the change spec's own status flip to the orchestrator.

## Definition of done

- [ ] The KeyManager section and the KMS inventory row match the change spec's two Proposed changes blocks verbatim in substance (DER→raw sign, local verify, no KMS Verify, RFC 7518 JWK covering P-256/384/521).
- [ ] The page's `**Date:**` is bumped; no `.specs/README.md` edit and no change-spec status flip are made here (orchestrator-owned).
- [ ] Cross-check: every claim added to the page is backed by code merged in tasks 01/03/04 (no spec statement describes unbuilt behaviour).
- [ ] Meets the repo definition of done for a docs change (prose states the why and what changed at the architecture level — see plan.md baseline; no code tests apply).
- [ ] Reviewable: open 02-ports-and-adapters.md and confirm the KeyManager note and KMS inventory row read as the change spec's Proposed changes, with the KMS `Verify` capability removed and P-521 listed.

## Open questions

- None.
