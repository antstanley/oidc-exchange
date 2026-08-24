# Task 10 — Canonical synchronization and merge housekeeping

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted (done certificates are forbidden)

**Implements:** source spec §Affected spec pages, §Type changes, and §Merge plan.
**Depends on:** 07 · exchange_refresh_rotation_flow; 08 · family_sid_and_revocation; 09 · session_reaper_and_internal_cleanup
**Produces:** canonical prose/schema reflecting delivered behaviour, logical-model synchronization, merged source spec location/status, and `.specs` indexes.
**Pointers:** `.specs/service/specs/{00-overview,01-domain-model,02-ports-and-adapters,03-service-flows,04-http-api,06-configuration,08-persistence}.md`; `.specs/service/specs/canonical-types.schema.json`; `schemas/datamodel.schema.json`; `.specs/changes/`; `.specs/README.md`.

## Steps

- [x] Apply each source-spec canonical block only after its behavior/tests ship; update page dates to merge date and replace—not duplicate—superseded refresh/sid/scheduler decisions.
- [x] Fold `Session`, `RetiredRefreshToken`, `TokenResponse`, `AccessTokenClaims.sid`, and closed audit enum changes into canonical types; mirror logical session/retired model changes without incorrectly closing its intentionally open event-type string.
- [x] Verify JSON syntax/schema references and prose links; check canonical pages describe the exact adapter and runtime behavior actually delivered.
- [x] Mark source spec Merged with merge date, move it to `changes/merged/`, and update its Change specs row; retain this plan in Plans as Planned/Done according to actual completion.
- [x] Confirm no done-certificate files exist for this plan and record the intentional omission in final task/plan evidence.

## Definition of done

- [x] Every affected canonical target named by the source spec is updated exactly once or explicitly proven unchanged by the source (for example logical-model open `event_type`).
- [x] JSON parses, all relative links resolve, no superseded reusable/hash-sid decision remains, and no aspirational change leaks into canonical text before implementation.
- [x] Source-spec status/location and `.specs/README.md` indexes agree.
- [x] No `*-certificate.md` is created anywhere in this plan directory.
- [x] Done certificates remain intentionally absent.

## Completion notes

- **Folded (page date → 2026-08-22 on each):** 00-overview (rotating-refresh Decision replaces the reusable-token Decision; external-scheduler Assumption replaced by the owned reaper + internal endpoint), 01-domain-model (`fam_` ID-scheme rows, Session family fields, RetiredRefreshToken entity, TokenResponse rotation semantics, required six-field AccessTokenClaims with family `sid`, `RefreshTokenReuse` variant, generation lifecycle diagram, classify/rotate/revoke-family query patterns), 02-ports-and-adapters (nine-method port + `RefreshResolution` + SR1–SR5 table + conformance-suite section; adapter-inventory rows updated to the shipped storage layouts), 03-service-flows (state-transition refresh flow with classification/grace/reuse/rotation-disabled; family-scoped access-token revocation; `build_access_token` `sid` clause; five new Decisions replace "*Refresh does not rotate*"), 04-http-api (`POST /internal/sessions/cleanup` row; Bootstrap step 7 reaper lifecycle with Lambda exclusion), 06-configuration (rotation keys in `[token]`, `[session_repository] cleanup_interval`, defaults summary, a Validation-at-load section), 08-persistence (Dynamo retirement items, consistent reads, user-item roster; SQL `retired_refresh_tokens` + atomic rotation; LMDB four databases + 256-key batched sweep; Valkey Lua rotation + counter clamp).
- **Schemas.** `canonical-types.schema.json`: Session gains required `family_id`/`generation` plus optional `rotated_at`; `RetiredRefreshToken` added; `TokenResponse.refresh_token` description states rotation semantics; `AccessTokenClaims.sid` added to `required` with pattern `^fam_[0-9a-z]{26}$`; `AuditEventType` enum closes over `refresh_token_reuse`. `schemas/datamodel.schema.json`: Session/RetiredRefreshToken mirrored in untyped form; its `event_type` stays an intentionally-open string — proven unchanged for the enum fold, exactly as the source requires.
- **Adaptations (all recorded here, none silent).**
  - The sibling `2026-08-05-validate_revoke_token_claims.md` is *not* merged on this branch (vendored-seam PR #19), so: the reuse-rejection event in folded Revocation text is this branch's shipped `ValidationFailed` (debug, fixed reason) rather than the sibling's `AuthenticationFailed`; the "*`sid` is the session's family identifier*" Decision drops its supersession cross-reference to a decision that never merged here; and the source spec's three same-directory links to that sibling (plus runtime-parity) are left pointing at their final merged location — they resolve when those specs land.
  - The 00-overview Assumption block's link was written as `specs/04-http-api.md`; corrected to `04-http-api.md` so it resolves from the page's own directory.
  - The moved source spec's relative links were rewritten one level deeper (`../service/…` → `../../service/…`, sibling specs to same-dir) so every navigational link resolves from `changes/merged/`.
- **Merge housekeeping.** Source spec header now reads `Status: Merged · Merged: 2026-08-22` and the file lives at `changes/merged/2026-08-05-rotate_refresh_tokens_with_reuse_detection.md`; the Change-specs row in `.specs/README.md` points there as Merged (listed first among merged entries by date); the Plans row stays Done and its source-spec link was re-pointed at the merged location, as was plan.md's own header link.
- **Verification (programmatic).** Both JSON schemas parse; key assertions hold (audit enum contains `refresh_token_reuse`, `sid` required with the exact pattern, `datamodel` keeps open `event_type`); every relative link in every touched canonical page, both schemas' `$ref`s, `.specs/README.md`, plan.md, and all ten done-task files resolves on disk — the only non-resolving references are the two documented not-yet-present sibling specs; no `*certificate*` file exists anywhere under the plan directory.
- Gates at commit: nextest workspace 468 passed / 50 skipped (docs-only change, run to confirm nothing drifted); fmt and clippy `-D warnings` clean.
