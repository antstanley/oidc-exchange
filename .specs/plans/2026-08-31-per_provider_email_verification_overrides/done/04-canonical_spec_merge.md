# Task 04 — Canonical spec merge

**Plan:** [plan.md](../plan.md) · **Certificate:** [04-canonical_spec_merge-certificate.md](04-canonical_spec_merge-certificate.md)

**Implements:** change spec [§Affected spec pages, §Proposed changes (all nine blocks), §Type changes, and §Merge plan](../../../changes/2026-08-31-per_provider_email_verification_overrides.md)
**Depends on:** 02 (review — the merged 05 section's precedence statements must be verifiable against the shipped derivation), 03 (review — the validation, warning, and Entra-recipe statements must be verifiable against the shipped lift, and the recipe must boot through the task-03 fixture)
**Produces:** service pages 01/03/05/06 and the sidecar describe the shipped behaviour; the change spec is `Merged`, dated, moved to `changes/merged/`, and re-indexed.
**Pointers:** [05-provider-system.md](../../../service/specs/05-provider-system.md) (§Tiers at `:9`, §OidcProvider behaviour at `:84`, insert §Email-verification overrides before §Provider registry at `:118`, Decisions at `:146`); [06-configuration.md](../../../service/specs/06-configuration.md) (§`[providers.<name>]` at `:269`); [03-service-flows.md](../../../service/specs/03-service-flows.md) (§Token exchange step 4 within `:12-110`, the *Registration demands a verified email* Decision within `:366-458`); [01-domain-model.md](../../../service/specs/01-domain-model.md) (§Token types at `:122`, §OidcProviderConfig at `:199`); `.specs/service/specs/canonical-types.schema.json`; `.specs/README.md`; `.specs/changes/2026-08-31-per_provider_email_verification_overrides.md` → `.specs/changes/merged/`

## Steps

- [x] Apply the nine Proposed-changes blocks verbatim to the four pages (05: Tiers pointer, `validate_id_token` bullet, new §Email-verification overrides, three Decisions; 06: the `[providers.<name>]` paragraph swap; 03: the two step-4 bullets and the reworded Decision; 01: the `IdentityClaims` bullet and the `OidcProviderConfig` field enumeration); bump each page's `**Date:**` to the merge date.
- [x] Fold the Type-changes fragment into `.specs/service/specs/canonical-types.schema.json`: add the `EmailVerification` `$def`, replace the `OidcProviderConfig` `$def` wholesale with the fragment's, drop the change-tracking `$comment`s, and keep the `NonEmptyString` `$ref` pointing at the global schema.
- [x] Verify the merged 05 section against the shipped code (Merge plan step 3): the precedence rule against the task-02 wiremock matrix, the validation errors and startup warning against task 03, and the Entra recipe TOML booting through `resolve_config_toml` via the task-03 fixture.
- [x] Flip the change spec to `**Status:** Merged`, add `**Merged:** <date>`, move it to `.specs/changes/merged/`, and fix its now-relative links (`../service/…` → `../../service/…`; `merged/2026-07-01-…` → `2026-07-01-…`).
- [x] Update `.specs/README.md`: move the change-spec row from the pending table entries to Merged status with the `changes/merged/` path.

## Definition of done

- [x] All nine blocks are landed and every cross-reference resolves, including the `#email-verification-overrides` anchor linked from 01, 03, and 06.
- [x] The sidecar parses as valid JSON; the `EmailVerification` `$def` is present and the `OidcProviderConfig` diff against the previous sidecar is exactly the added optional `email_verification` property.
- [x] Every statement the merged 05 section makes is true of the shipped code — precedence, the four validation-error classes, the single startup warning, and the Entra recipe booting (Merge plan step 3, evidenced by the task-02/03 test names).
- [x] The change spec sits in `changes/merged/` with status flipped and internal links fixed, and the `.specs/README.md` row is updated.
- [x] Meets the repo definition of done for the touched surfaces (docs and schema only — the link and JSON checks above stand in for the code gates; no code changes in this task).
- [x] Reviewable: a reviewer opens the merged 05 §Email-verification overrides, follows each cross-link from 01, 03, and 06 to its anchor, and re-runs the Entra fixture test the section's recipe corresponds to.
