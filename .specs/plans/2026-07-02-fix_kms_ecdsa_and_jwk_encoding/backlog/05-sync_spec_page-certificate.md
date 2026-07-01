# Done Certificate — Task 05: Sync canonical spec page

**Task:** [05-sync_spec_page.md](05-sync_spec_page.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified

> This certificate is a verification protocol for Task 05. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 05) ≡ every obligation O1…O5 below holds, each backed by the evidence the obligation
names (a file location or a text comparison) — not by assertion.

## Premises

- **P1 — Goal.** Canonical page `.specs/service/specs/02-ports-and-adapters.md` describes the shipped adapter: ES\* DER→raw on `sign`, local `verify` against the cached SPKI, and RFC 7518 JWK output covering P-256/384/521, with the KMS inventory row updated.
- **P2 — Obligations.** The task is done iff O1…O5 all hold; O5 is the Reviewable item.
- **P3 — Invariants.** Must not alter unrelated sections of the page (other port traits, other adapter rows), and must not touch `.specs/README.md` or the change spec's status (orchestrator-owned).

## Obligations

- **O1 — KeyManager section and KMS inventory row match the Proposed changes.**
  - *Claim:* the KeyManager note states DER→raw on `sign`, RS/PS pass-through, and local `verify` against the cached SPKI (raw `r || s`, no raw→DER); the KMS inventory row reads `RS/PS/ES 256/384/512; ECDSA DER→raw JWS conversion on sign; local verify against the cached public key; JWK cached on OnceCell; Sign/GetPublicKey`.
  - *Evidence to collect:* read `.specs/service/specs/02-ports-and-adapters.md` KeyManager section (was `:42-54`) and the KMS inventory row (was `:87`); compare against the change spec's two Proposed changes blocks. Confirm the row no longer lists KMS `Verify`.
  - *Status:* ☐ unverified

- **O2 — RFC 7517/7518 JWK note present; Date bumped; no README/status edits.**
  - *Claim:* a note states RSA `n`/`e` are Base64urlUInt with no leading zeros (`e = 65537` → `AQAB`) and EC keys cover P-256/P-384/P-521; the page `**Date:**` is 2026-07-02; `.specs/README.md` and the change-spec status are untouched.
  - *Evidence to collect:* read the text after the inventory table and the `**Date:**` line (`:3`). Confirm no diff to `.specs/README.md` and no change to the change spec's `**Status:**`.
  - *Status:* ☐ unverified

- **O3 — Every added claim is backed by merged code.**
  - *Claim:* no statement added to the page describes unbuilt behaviour — DER→raw sign, local verify, RFC 7518 `n`/`e`, and P-521 JWK all correspond to code merged in tasks 01/03/04/02.
  - *Evidence to collect:* for each added claim, name the merged code that implements it (`sign` ES\* branch, `verify` local path, RSA strip helper, ES512 arm). Flag any claim with no corresponding code.
  - *Status:* ☐ unverified

- **O4 — Meets the repo definition of done for a docs change.**
  - *Claim:* the prose states the why and what changed at the architecture level; no code tests apply to a spec-only edit.
  - *Evidence to collect:* read the edited sections and confirm they convey the architectural change (contract of `sign`/`verify`, JWK compliance) rather than restating implementation line-by-line.
  - *Status:* ☐ unverified

- **O5 — Reviewable: page reads as the Proposed changes, Verify removed, P-521 listed.**
  - *Claim:* a reviewer opens 02-ports-and-adapters.md and sees the KeyManager note and KMS inventory row as the change spec's Proposed changes, with KMS `Verify` removed and P-521 present.
  - *Evidence to collect:* open the page; read the KeyManager section and the KMS row; confirm `Verify` is absent from the row's capability list and `P-521` appears in the EC-coverage note.
  - *Status:* ☐ unverified

## Regression check

- Other rows of the Adapter inventory table and other port-trait subsections → unchanged after the edit : ☐ (PRESERVED / REGRESSION)

## Residue

- Outside the DoD: flipping the change spec's `**Status:**` to `Merged`, moving it to `.specs/changes/merged/`, and updating `.specs/README.md` are the orchestrator's Merge-plan steps, not this task's.

## Conclusion

VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
