# Task 05 — Website providers guide

**Plan:** [plan.md](../plan.md) · **Certificate:** [05-website_providers_guide-certificate.md](05-website_providers_guide-certificate.md)

**Implements:** change spec [§Implementation notes step 5 and the *The website recipe is part of the fix* Decision](../../../changes/2026-08-31-per_provider_email_verification_overrides.md)
**Depends on:** 03 (review — the documented keys must exist and the documented recipe must be the same shape as a passing boot fixture; the guide currently reproduces issue #48)
**Produces:** the providers guide documents both override keys in its field table and ships a Microsoft Entra ID recipe that can admit a user, with the `xms_edov` enablement note and the `trust_email_verified` fallback caveat.
**Pointers:** `apps/website/src/content/docs/guides/providers.md:34-44` (the `[providers.<name>]` field table), `:50-59` (the Microsoft Entra ID example); `.specs/website/specs/00-overview.md` (unchanged — it inventories the docs tree at section level, so a content edit inside `guides/providers.md` needs no website spec change)

## Steps

- [ ] Add `email_verified_claim` and `trust_email_verified` rows to the field table: oidc-adapter only, optional, their types and defaults, the mutual exclusivity, and that an explicit `email_verified` claim from the provider always takes precedence.
- [ ] Extend the Microsoft Entra ID example with `email_verified_claim = "xms_edov"` and prose covering: why the override is needed (v2.0 id_tokens carry `email` but no `email_verified`, so the default mode denies every sign-in), the enablement note (`xms_edov` is off by default and enabled per app registration by a tenant administrator), and the `trust_email_verified = true` fallback with the user-mutable caveat and the prefer-the-claim-mapping guidance.
- [ ] Keep the recipe TOML consistent with the canonical 05 recipe and with the task-03 Entra boot fixture (same adapter, issuer shape, scopes, and override key).
- [ ] Make no website spec page changes — this is a content-only edit.

## Definition of done

- [ ] The table rows and recipe match the shipped semantics exactly: key names, value types, defaults, mutual exclusivity, and explicit-claim precedence.
- [ ] The documented Entra block is the same shape as the passing `resolve_config_toml` fixture from task 03 — the guide no longer documents a block that cannot admit a user.
- [ ] Meets the repo definition of done for the website workspace (`pnpm format:check`, `pnpm lint`, `pnpm typecheck` including `astro check` — see plan.md baseline).
- [ ] Reviewable: a reviewer reads the rendered guide section (or its diff), cross-checks each table row against the merged 05 table, and pastes the documented recipe against the task-03 boot fixture to confirm the shapes agree.
