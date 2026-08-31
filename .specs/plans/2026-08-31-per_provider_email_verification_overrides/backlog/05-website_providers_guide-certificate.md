# Done Certificate — Task 05: Website providers guide

**Task:** [05-website_providers_guide.md](05-website_providers_guide.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-08-31 — unverified   <!-- validator sets: Validated YYYY-MM-DD -->

> This certificate is a verification protocol for Task 05. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 05) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names (a file location, a rendered diff, or a gate result) — not by assertion.

## Premises

- **P1 — Goal.** The providers guide documents both override keys in its field table and
  ships a Microsoft Entra ID recipe that can admit a user, with the `xms_edov` enablement
  note and the `trust_email_verified` fallback caveat.
- **P2 — Obligations.** Done iff O1…O4 all hold. One Oi per definition-of-done item, in DoD
  order; O4 is the Reviewable item.
- **P3 — Invariants.** Must not change any other row of the field table, any other provider
  example, or any website spec page (`.specs/website/specs/00-overview.md` inventories the
  docs tree at section level, so it stays untouched).

## Obligations

- **O1 — Table rows and recipe match the shipped semantics.**
  - *Claim:* `apps/website/src/content/docs/guides/providers.md` carries table rows for
    `email_verified_claim` and `trust_email_verified` stating: oidc-adapter only, optional,
    string / boolean types, `trust_email_verified` default `false`, mutual exclusivity, and
    that an explicit `email_verified` claim from the provider always takes precedence.
  - *Evidence to collect:* read the two rows and check each stated fact against the shipped
    lift (`provider_config_to_oidc`) and derivation (Task 02's precedence rule) — expect no
    contradiction with any Task 02/03 test expectation; confirm the prose covers the
    `xms_edov` enablement note (off by default, tenant-admin, per app registration) and the
    user-mutable caveat with the prefer-claim-mapping guidance.
  - *Status:* ☐ unverified

- **O2 — The documented Entra block is the shape of a passing fixture.**
  - *Claim:* the guide's Entra example includes `email_verified_claim = "xms_edov"` and has
    the same shape (adapter, v2.0 issuer, scopes, override key) as the Task 03
    `resolve_config_toml` fixture that passes — the guide no longer documents a block that
    cannot admit a user.
  - *Evidence to collect:* compare the example TOML with the Task 03 Entra fixture key by key
    (placeholder syntax `${VAR}` versus literal values aside) — expect matching shape; run
    the Task 03 Entra test — expect PASS.
  - *Checks:* confirm the example and the merged 05 recipe (Task 04) agree on key name and
    claim name — `email_verified_claim = "xms_edov"`, not a variant spelling.
  - *Status:* ☐ unverified

- **O3 — Meets the repo definition of done for the website workspace.**
  - *Claim:* the website workspace's gates pass.
  - *Evidence to collect:* run `pnpm format:check`, `pnpm lint`, and `pnpm typecheck`
    (`astro check`) for `apps/website` — expect all clean (per
    [development-guidelines.md](../../../development-guidelines.md) §Definition of done).
  - *Status:* ☐ unverified

- **O4 — Reviewable: read the section, cross-check the table, paste the recipe (Reviewable).**
  - *Claim:* a reviewer reads the rendered guide section (or its diff), cross-checks each new
    table row against the merged 05 table, and pastes the documented recipe against the
    Task 03 boot fixture to confirm the shapes agree.
  - *Evidence to collect:* perform the read and the row-by-row cross-check; record the
    recipe-versus-fixture comparison.
  - *Status:* ☐ unverified

## Regression check

- The rest of `guides/providers.md` (the Google/GitHub examples, the surrounding field-table
  rows, the endpoint-origins prose): diff the file — expect no hunk outside the two new rows
  and the Entra example block : ☐ (PRESERVED / REGRESSION)
- The website build: `astro check` over `apps/website` — expect no new diagnostics :
  ☐ (PRESERVED / REGRESSION)

## Residue

Notes for the validator: Task 04 may land before or after this task (both depend on 03, not
on each other); if 04 has not merged yet, O2's cross-check against the merged 05 recipe
falls back to the change spec's §Proposed changes recipe block — the two are defined to be
identical.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
