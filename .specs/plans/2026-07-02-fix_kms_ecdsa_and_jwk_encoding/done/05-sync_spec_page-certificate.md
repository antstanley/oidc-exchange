# Done Certificate — Task 05: Sync canonical spec page

**Task:** [05-sync_spec_page.md](05-sync_spec_page.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

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
  - *Evidence:* new paragraph at `02-ports-and-adapters.md:55-61` is word-for-word the change spec's first Proposed changes block (`2026-07-01-fix_kms_ecdsa_and_jwk_encoding.md:44-50`); the KMS row at `:95` is word-for-word the second block's row (`:54`) — capability list ends `` `Sign`/`GetPublicKey` ``, `Verify` removed.
  - *Status:* ☑ SATISFIED

- **O2 — RFC 7517/7518 JWK note present; Date bumped; no README/status edits.**
  - *Claim:* a note states RSA `n`/`e` are Base64urlUInt with no leading zeros (`e = 65537` → `AQAB`) and EC keys cover P-256/P-384/P-521; the page `**Date:**` is 2026-07-02; `.specs/README.md` and the change-spec status are untouched.
  - *Evidence to collect:* read the text after the inventory table and the `**Date:**` line (`:3`). Confirm no diff to `.specs/README.md` and no change to the change spec's `**Status:**`.
  - *Evidence:* RFC note at `02-ports-and-adapters.md:106-108` states Base64urlUInt with no leading zero octets, `e = 65537` → `AQAB`, and EC coverage of P-256/P-384/P-521; `**Date:**` at `:3` reads `2026-07-02`. `jj diff --stat` touches only `02-ports-and-adapters.md` (14+/2−); `.specs/README.md` untouched; change spec `**Status:**` still `Proposed` (`2026-07-01-fix_kms_ecdsa_and_jwk_encoding.md:3`).
  - *Status:* ☑ SATISFIED

- **O3 — Every added claim is backed by merged code.**
  - *Claim:* no statement added to the page describes unbuilt behaviour — DER→raw sign, local verify, RFC 7518 `n`/`e`, and P-521 JWK all correspond to code merged in tasks 01/03/04/02.
  - *Evidence to collect:* for each added claim, name the merged code that implements it (`sign` ES\* branch, `verify` local path, RSA strip helper, ES512 arm). Flag any claim with no corresponding code.
  - *Evidence:* all merged in ancestor commits (kms 01–04). DER→raw on sign: `sign` calls `signature_to_jws_form` (`crates/adapters/src/kms/mod.rs:416,160-162`) → `der_to_raw_ecdsa` via `Signature::from_der` per curve (`:122-134`), RS/PS pass through (test `:789`). Local verify, no KMS call: `verify` (`:430-441`) uses cached SPKI from the `OnceCell<(Vec<u8>, serde_json::Value)>` (`:22`) and `verify_locally` (`:364`) with `Signature::from_slice` — no raw→DER (`:307-310`). RFC 7518 `n`/`e`: `base64url_uint` (`:185`) with `AQAB` tests (`:654-686`). P-521: `ES512` arm in `parse_spki_to_jwk` (`:217-222`, `EC_COORD_LEN_P521` `:177`) and `verify_ecdsa_p521` (`:343`). No unbacked claim found.
  - *Status:* ☑ SATISFIED

- **O4 — Meets the repo definition of done for a docs change.**
  - *Claim:* the prose states the why and what changed at the architecture level; no code tests apply to a spec-only edit.
  - *Evidence to collect:* read the edited sections and confirm they convey the architectural change (contract of `sign`/`verify`, JWK compliance) rather than restating implementation line-by-line.
  - *Evidence:* the KeyManager paragraph states the port contract (JWS wire form is `sign`'s output form; `verify` is in-process with a stated why — no KMS round-trip on revoke) and the RFC note states the compliance guarantee and its consequence (every signing algorithm has a published JWK at `/keys`); no crate names or line-level detail leak into the page. Docs-only change, no code tests apply.
  - *Status:* ☑ SATISFIED

- **O5 — Reviewable: page reads as the Proposed changes, Verify removed, P-521 listed.**
  - *Claim:* a reviewer opens 02-ports-and-adapters.md and sees the KeyManager note and KMS inventory row as the change spec's Proposed changes, with KMS `Verify` removed and P-521 present.
  - *Evidence to collect:* open the page; read the KeyManager section and the KMS row; confirm `Verify` is absent from the row's capability list and `P-521` appears in the EC-coverage note.
  - *Evidence:* exercised — opened the page: KeyManager section (`:55-61`) and KMS row (`:95`) read as the Proposed changes; the row's capability list is `` `Sign`/`GetPublicKey` `` with no `Verify`; `P-521` appears in the EC-coverage note (`:107-108`).
  - *Status:* ☑ SATISFIED

## Regression check

- Other rows of the Adapter inventory table and other port-trait subsections → unchanged after the edit : ☑ PRESERVED — `jj diff` shows exactly four hunks in the one file (Date line, KeyManager paragraph insertion, the KMS row, the RFC note insertion); all other inventory rows and port-trait subsections are untouched.

## Residue

- Outside the DoD: flipping the change spec's `**Status:**` to `Merged`, moving it to `.specs/changes/merged/`, and updating `.specs/README.md` are the orchestrator's Merge-plan steps, not this task's.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: Both Proposed changes blocks land verbatim on 02-ports-and-adapters.md (DER→raw sign, local verify, KMS `Verify` dropped, RFC 7517/7518 note with P-521), the Date is bumped to 2026-07-02, every added claim traces to merged kms 01–04 code, and nothing outside the page (other rows/sections, `.specs/README.md`, the change spec's status) was touched.
