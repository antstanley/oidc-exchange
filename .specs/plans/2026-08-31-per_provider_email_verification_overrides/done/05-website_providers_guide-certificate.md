# Done Certificate — Task 05: Website providers guide

**Task:** [05-website_providers_guide.md](05-website_providers_guide.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-08-31

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
  - *Status:* ☑ SATISFIED — providers.md:45-46 carry both rows; every stated fact traced to code:
    oidc-only lift in `provider_config_to_oidc` → `lift_email_verification` (bootstrap.rs:1666-1720:
    string/non-empty/≤64 code points via `MAX_EMAIL_VERIFIED_CLAIM_LEN`:1656, bool required with
    `false` ≡ absent :1684-1695, mutual exclusion :1675-1682); bool-or-string via `coerce_bool`
    (adapters shared/claims.rs:14-23); explicit-claim precedence both directions in
    `derive_email_verified` (oidc/mod.rs:143-145, pinned by the explicit-false tests at :1801, :1858).
    Prose paragraph (providers.md:50) matches: both-keys config error at registry build, one
    structured startup warning (`warn_nonstandard_email_verification` bootstrap.rs:1623-1651,
    called at :1602). The `xms_edov` enablement note (off by default, tenant admin, per app
    registration) is at providers.md:66; the user-mutable caveat with prefer-claim-mapping
    guidance at providers.md:68. No contradiction with any Task 02/03 test expectation.

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
  - *Status:* ☑ SATISFIED — providers.md:57-63 vs the Task 03 fixture
    `entra_shaped_block_with_mapped_claim_resolves` (bootstrap.rs:2321-2352): same shape key by
    key — `[providers.entra]`, `adapter = "oidc"`, v2.0 `login.microsoftonline.com` issuer
    (`${ENTRA_TENANT_ID}` placeholder vs the fixture's literal `common` tenant; `${VAR}`
    interpolation applies to every config string via `resolve_placeholders`, bootstrap.rs:264-266),
    `scopes = ["openid", "email", "profile"]`, `email_verified_claim = "xms_edov"`. Ran
    `cargo test -p oidc-exchange entra_shaped_block_with_mapped_claim_resolves` → PASS (1 passed).
    Task 04 has not merged (parent ccca754a is "task 03 done"), so per Residue the cross-check
    fell back to the change spec §Proposed changes recipe block
    (2026-08-31-per_provider_email_verification_overrides.md:210-217) — byte-identical to the
    documented example; key and claim names agree exactly.

- **O3 — Meets the repo definition of done for the website workspace.**
  - *Claim:* the website workspace's gates pass.
  - *Evidence to collect:* run `pnpm format:check`, `pnpm lint`, and `pnpm typecheck`
    (`astro check`) for `apps/website` — expect all clean (per
    [development-guidelines.md](../../../development-guidelines.md) §Definition of done).
  - *Status:* ☑ SATISFIED — ran in the task workspace's `apps/website`:
    `pnpm format:check` → "All matched files use the correct format" (exit 0);
    `pnpm lint` (`oxlint --deny-warnings`) → clean (exit 0);
    `pnpm typecheck` (`astro check`) → 0 errors, 0 warnings, 0 hints (exit 0).

- **O4 — Reviewable: read the section, cross-check the table, paste the recipe (Reviewable).**
  - *Claim:* a reviewer reads the rendered guide section (or its diff), cross-checks each new
    table row against the merged 05 table, and pastes the documented recipe against the
    Task 03 boot fixture to confirm the shapes agree.
  - *Evidence to collect:* perform the read and the row-by-row cross-check; record the
    recipe-versus-fixture comparison.
  - *Status:* ☑ SATISFIED — read the full changed section in context (providers.md:30-79) and the
    diff (one file, +13/-4, single region). Row-by-row cross-check: `email_verified_claim` row —
    type/optionality/default, absence-only read, bool-or-string coercion, non-empty ≤64, mutual
    exclusion all match the lift and derivation; `trust_email_verified` row — boolean, default
    false, non-empty-email condition, explicit-false ≡ absent, mutual exclusion all match.
    Recipe-versus-fixture comparison recorded under O2: shapes agree on adapter, issuer shape,
    scopes, and override key; the fixture asserts the lift resolves to
    `EmailVerification::Claim("xms_edov")` with scopes intact, so the documented block admits a
    user end to end (derive → `Some(true)` → `registration_policy_reason` passes,
    core service/exchange.rs:96-115).

## Regression check

- The rest of `guides/providers.md` (the Google/GitHub examples, the surrounding field-table
  rows, the endpoint-origins prose): diff the file — expect no hunk outside the two new rows
  and the Entra example block : PRESERVED — `jj diff --git` shows a single region (table rows,
  precedence paragraph, Entra example and its two paragraphs); Google/GitHub/Apple examples and
  all pre-existing table rows untouched; grep finds no remaining `providers.microsoft` /
  `MICROSOFT_CLIENT_*` reference anywhere in the docs, so the example rename leaves nothing
  dangling. `.specs/website/specs/00-overview.md` untouched (diff is one file).
- The website build: `astro check` over `apps/website` — expect no new diagnostics :
  PRESERVED — 0 errors, 0 warnings, 0 hints.

## Residue

Notes for the validator: Task 04 may land before or after this task (both depend on 03, not
on each other); if 04 has not merged yet, O2's cross-check against the merged 05 recipe
falls back to the change spec's §Proposed changes recipe block — the two are defined to be
identical.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1-O4 all SATISFIED with collected evidence (every documented fact traced to the shipped
lift/derivation code, the recipe matches the passing Task 03 fixture and the change spec recipe
byte for byte, and all three website gates plus the fixture test ran clean), and both regression
checks are PRESERVED on a one-file docs-only diff.
