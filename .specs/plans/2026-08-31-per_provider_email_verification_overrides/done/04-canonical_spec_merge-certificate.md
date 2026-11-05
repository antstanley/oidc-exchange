# Done Certificate — Task 04: Canonical spec merge

**Task:** [04-canonical_spec_merge.md](04-canonical_spec_merge.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-08-31

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
  - *Status:* SATISFIED — full `jj diff --git` read against parent ccca754a: all nine blocks
    match the change spec's §Proposed changes word-for-word (blockquote markers stripped,
    rewrap only); 05 §Email-verification overrides sits at `:123`, between §OidcProvider
    behaviour and §Provider registry; 06's bare "See 05" sentence deleted and the new
    paragraph is the `[providers.<name>]` section's second paragraph (`:269`–`:297`); 03's
    Found-active bullet drops the allowlist-conditional framing; anchor links resolve —
    01 ×2, 03 ×2, 06 ×1, plus 05-internal ×2 — against the `## Email-verification overrides`
    heading; Dates on 01/03/05/06 all bumped to 2026-08-31.

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
  - *Status:* SATISFIED — `python3 -m json.tool` parses the sidecar; programmatic diff vs
    `jj file show -r @-`: added `$defs` = [`EmailVerification`] only, removed = none,
    changed = [`OidcProviderConfig`] and only by the added optional `email_verification`
    property (all shared properties byte-equal); both the `$def` and the property match the
    change spec's Type-changes fragment exactly with the change-tracking `$comment`s dropped;
    `../../canonical-types.schema.json#/$defs/NonEmptyString` resolves from
    `.specs/service/specs/` to `.specs/canonical-types.schema.json`, which carries
    `NonEmptyString`.

- **O3 — The merged 05 section is true of the shipped code.**
  - *Claim:* every statement in §Email-verification overrides holds: the two-step precedence
    rule, the three modes' derivations, the four validation-error classes, the
    single startup warning, and the Entra recipe.
  - *Evidence to collect:* map each statement to its shipped evidence — the Task 02 wiremock
    matrix for precedence and modes, the Task 03 rejection cases for validation, the
    Task 03 warning call site, and the Task 03 Entra `resolve_config_toml` fixture for the
    recipe (the section's TOML and the fixture must be the same shape) — and record each
    test/case name against its sentence.
  - *Status:* SATISFIED — precedence and modes: `derive_email_verified`
    (crates/adapters/src/oidc/mod.rs:140–156) implements the two-step rule exactly as the
    table and paragraph state, and the Task 02 wiremock matrix passed 10/10
    (`claim_mode_derives_true_from_json_bool_override`, `claim_mode_coerces_string_true_override`,
    `claim_mode_absent_override_stays_none`, `claim_mode_non_coercible_override_stays_none`,
    `claim_mode_explicit_false_beats_true_override`, `trust_email_mode_present_email_derives_true`,
    `trust_email_mode_absent_email_stays_none`, `trust_email_mode_empty_email_stays_none`,
    `trust_email_mode_explicit_false_is_never_overturned`, `standard_mode_absent_claim_stays_none`).
    Four validation-error classes: bootstrap.rs 1678 (both set), 1686 (non-boolean trust),
    1702/1706 (non-string / empty claim), 1714 (>64 code points) — evidenced by
    `both_email_verification_keys_set_is_rejected_naming_the_provider`,
    `non_boolean_trust_email_verified_is_rejected_never_coerced`,
    `invalid_email_verified_claim_values_are_rejected_naming_the_provider`,
    `claim_name_exactly_at_the_cap_is_accepted_and_one_over_rejected`,
    `claim_name_cap_counts_code_points_not_bytes`. Single startup warning: bootstrap.rs
    1625–1650, one `tracing::warn!` per non-Standard mode naming provider and mode —
    `claim_mode_logs_exactly_one_warning_naming_provider_and_claim`,
    `trust_email_mode_logs_exactly_one_warning_naming_provider_and_mode`,
    `standard_mode_logs_no_warning`. Entra recipe: the fixture at bootstrap.rs:2321
    (`adapter = "oidc"`, v2.0 issuer, scopes `openid email profile`,
    `email_verified_claim = "xms_edov"`) is the same shape as the section's TOML and
    `entra_shaped_block_with_mapped_claim_resolves` PASSED. The 05 policy sentence is also
    true of the core: `registration_policy_reason` (exchange.rs:96–115) requires
    `Some(true)` unconditionally at all three call sites (331/349/425). 18/18 server tests
    passed.

- **O4 — Change spec merged, moved, links fixed; README row updated.**
  - *Claim:* the change spec sits at
    `.specs/changes/merged/2026-08-31-per_provider_email_verification_overrides.md` with
    `**Status:** Merged` and a `**Merged:**` date; its internal relative links resolve from
    the new location; the `.specs/README.md` change-spec row points at the merged path with
    Merged status.
  - *Evidence to collect:* confirm the file's location, header, and that no file remains at
    the old path; follow its `../../service/specs/…` links and its reference to the 2026-07-01
    Apple-coercion spec (now same-directory) — expect both to resolve; read the README row.
  - *Status:* SATISFIED — the file sits at
    `.specs/changes/merged/2026-08-31-per_provider_email_verification_overrides.md` with
    `**Status:** Merged · **Date:** 2026-08-31 · **Merged:** 2026-08-31`; nothing remains at
    the old path (`ls` → No such file); all six `../../service/specs/…` links resolve by `ls`
    and the References link now points same-directory at
    `2026-07-01-require_iss_aud_in_token_validation.md`, which exists in `merged/`; the
    README change-spec row points at the merged path with Merged status (and the Plans-table
    source-spec link was updated to the merged path).

- **O5 — Meets the repo definition of done for the touched surfaces.**
  - *Claim:* the change is docs-and-schema only, with the link and JSON checks standing in
    for code gates.
  - *Evidence to collect:* confirm the task's change set touches only `.specs/` markdown and
    the service sidecar (no code); re-run the O1 link resolution and O2 JSON parse as the
    mechanical gate.
  - *Status:* SATISFIED — `jj diff --git --stat` lists exactly seven files, all under
    `.specs/` (README.md, the moved change spec, pages 01/03/05/06, the sidecar); nothing
    under `.specs/plans/`, no code; the link-resolution and JSON-parse gates re-ran clean.

- **O6 — Reviewable: walk the merged section and its anchors, re-run the recipe's test (Reviewable).**
  - *Claim:* a reviewer opens the merged 05 §Email-verification overrides, follows each
    cross-link from 01, 03, and 06 to the anchor, and re-runs the Entra fixture test the
    recipe corresponds to.
  - *Evidence to collect:* perform the walk and record the three resolved links; run the
    named Task 03 Entra test — expect PASS.
  - *Status:* SATISFIED — opened the merged 05 §Email-verification overrides (`:123–:170`)
    and read it end to end; followed each cross-link to the anchor: 01 §Token types and
    §OidcProviderConfig, 03 step-4 Not-found bullet and the *Registration demands a verified
    email* Decision, 06 §`[providers.<name>]` — all five target
    `05-provider-system.md#email-verification-overrides`, whose heading exists and slugs to
    that anchor; re-ran `cargo nextest run -p oidc-exchange -E
    'test(entra_shaped_block_with_mapped_claim_resolves)'` singly → 1 passed.

## Regression check

- The unedited remainder of the four pages: diff each page and confirm no hunk falls outside
  the nine blocks and the date lines : PRESERVED — every hunk in the full diff sits inside a
  specified block or is a `**Date:**` bump; no stray edits.
- The sidecar's other `$def`s (`IdentityClaims` in particular, which the change spec says is
  not republished): diff — expect byte-identical : PRESERVED — programmatic `$defs`
  comparison vs the parent revision: only `EmailVerification` added and `OidcProviderConfig`
  changed (by exactly the one property); `IdentityClaims` and every other `$def` byte-identical.

## Residue

Notes for the validator: 02-ports-and-adapters.md is intentionally untouched (the
`IdentityProvider` port signature is unchanged) — its absence from the diff is correct, not
an omission. The plan's own README row (Plans table) is spec-planner housekeeping, not part
of this task's DoD.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O6 all SATISFIED with collected evidence — nine verbatim blocks, an exactly-additive
sidecar delta, 29 passing named tests tying every merged statement to shipped code, the change
spec moved/flipped/re-indexed, and the Reviewable walk exercised — with both regression checks
PRESERVED. (Validator note: references to the pre-move change-spec path remain inside
`.specs/plans/` task files and plan.md — outside this task's scope per the Residue; plan
close-out housekeeping.)
