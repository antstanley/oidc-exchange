# Done Certificate — Task 04: Canonical spec merge

**Task:** [04-canonical_spec_merge.md](04-canonical_spec_merge.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-08-31 — unverified   <!-- validator sets: Validated YYYY-MM-DD -->

> This certificate is a verification protocol for Task 04. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 04) ≡ every obligation O1…O6 below holds, each backed by the evidence the obligation
names (a file location, a rendered diff, or a test result) — not by assertion.

## Premises

- **P1 — Goal.** Service pages 01/03/05/06 and the sidecar describe the shipped behaviour;
  the change spec is `Merged`, dated, moved to `changes/merged/`, and re-indexed.
- **P2 — Obligations.** Done iff O1…O6 all hold. One Oi per definition-of-done item, in DoD
  order; O6 is the Reviewable item.
- **P3 — Invariants.** Must not alter any spec text outside the nine Proposed-changes blocks
  and the two sidecar `$def`s (plus page dates); every sidecar `$def` other than
  `OidcProviderConfig` and the new `EmailVerification` stays byte-identical.

## Obligations

- **O1 — All nine blocks landed; every cross-reference resolves.**
  - *Claim:* the four pages carry exactly the change spec's Proposed-changes text (05: the
    Tiers pointer sentence, the `validate_id_token` bullet extension, the new
    §Email-verification overrides between §OidcProvider behaviour and §Provider registry,
    three new Decisions; 06: the `[providers.<name>]` paragraph swap with the bare
    "See 05" sentence deleted; 03: the two rewritten step-4 bullets — the Found-active
    bullet without the allowlist-conditional framing — and the reworded Decision; 01: the
    `IdentityClaims` bullet and the `OidcProviderConfig` enumeration with
    `email_verification`), and the `#email-verification-overrides` anchor resolves from the
    links in 01, 03, and 06.
  - *Evidence to collect:* diff each edited section against the change spec's blocks — expect
    verbatim content (formatting-only rewrap acceptable); resolve each of the three anchored
    links to the new heading; confirm each page's `**Date:**` is bumped.
  - *Status:* ☐ unverified

- **O2 — Sidecar valid; diff is exactly the additive property.**
  - *Claim:* `.specs/service/specs/canonical-types.schema.json` parses as JSON, carries the
    `EmailVerification` `$def` (default `standard`, the three `oneOf` arms, the 64-cap claim
    object), and the `OidcProviderConfig` `$def` differs from its previous state only by the
    optional `email_verification` property; the change-tracking `$comment`s are dropped.
  - *Evidence to collect:* parse the file (e.g. `python3 -m json.tool`) — expect success;
    diff the `OidcProviderConfig` `$def` against the pre-merge version — expect one added
    property and no other change; confirm the fragment's `$comment`s are absent.
  - *Checks:* resolve the `NonEmptyString` `$ref` from the claim object — confirm it targets
    the repo-global `canonical-types.schema.json` `$defs/NonEmptyString` at the correct
    relative depth from the service sidecar.
  - *Status:* ☐ unverified

- **O3 — The merged 05 section is true of the shipped code.**
  - *Claim:* every statement in §Email-verification overrides holds: the two-step precedence
    rule, the three modes' derivations, the four validation-error classes, the
    single startup warning, and the Entra recipe.
  - *Evidence to collect:* map each statement to its shipped evidence — the Task 02 wiremock
    matrix for precedence and modes, the Task 03 rejection cases for validation, the
    Task 03 warning call site, and the Task 03 Entra `resolve_config_toml` fixture for the
    recipe (the section's TOML and the fixture must be the same shape) — and record each
    test/case name against its sentence.
  - *Status:* ☐ unverified

- **O4 — Change spec merged, moved, links fixed; README row updated.**
  - *Claim:* the change spec sits at
    `.specs/changes/merged/2026-08-31-per_provider_email_verification_overrides.md` with
    `**Status:** Merged` and a `**Merged:**` date; its internal relative links resolve from
    the new location; the `.specs/README.md` change-spec row points at the merged path with
    Merged status.
  - *Evidence to collect:* confirm the file's location, header, and that no file remains at
    the old path; follow its `../../service/specs/…` links and its reference to the 2026-07-01
    Apple-coercion spec (now same-directory) — expect both to resolve; read the README row.
  - *Status:* ☐ unverified

- **O5 — Meets the repo definition of done for the touched surfaces.**
  - *Claim:* the change is docs-and-schema only, with the link and JSON checks standing in
    for code gates.
  - *Evidence to collect:* confirm the task's change set touches only `.specs/` markdown and
    the service sidecar (no code); re-run the O1 link resolution and O2 JSON parse as the
    mechanical gate.
  - *Status:* ☐ unverified

- **O6 — Reviewable: walk the merged section and its anchors, re-run the recipe's test (Reviewable).**
  - *Claim:* a reviewer opens the merged 05 §Email-verification overrides, follows each
    cross-link from 01, 03, and 06 to the anchor, and re-runs the Entra fixture test the
    recipe corresponds to.
  - *Evidence to collect:* perform the walk and record the three resolved links; run the
    named Task 03 Entra test — expect PASS.
  - *Status:* ☐ unverified

## Regression check

- The unedited remainder of the four pages: diff each page and confirm no hunk falls outside
  the nine blocks and the date lines : ☐ (PRESERVED / REGRESSION)
- The sidecar's other `$def`s (`IdentityClaims` in particular, which the change spec says is
  not republished): diff — expect byte-identical : ☐ (PRESERVED / REGRESSION)

## Residue

Notes for the validator: 02-ports-and-adapters.md is intentionally untouched (the
`IdentityProvider` port signature is unchanged) — its absence from the diff is correct, not
an omission. The plan's own README row (Plans table) is spec-planner housekeeping, not part
of this task's DoD.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
