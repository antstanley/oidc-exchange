# Task 10 — Canonical synchronization and merge housekeeping

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted (done certificates are forbidden)

**Implements:** source spec §Affected spec pages, §Type changes, and §Merge plan.
**Depends on:** 07 · exchange_refresh_rotation_flow; 08 · family_sid_and_revocation; 09 · session_reaper_and_internal_cleanup
**Produces:** canonical prose/schema reflecting delivered behaviour, logical-model synchronization, merged source spec location/status, and `.specs` indexes.
**Pointers:** `.specs/service/specs/{00-overview,01-domain-model,02-ports-and-adapters,03-service-flows,04-http-api,06-configuration,08-persistence}.md`; `.specs/service/specs/canonical-types.schema.json`; `schemas/datamodel.schema.json`; `.specs/changes/`; `.specs/README.md`.

## Steps

- [ ] Apply each source-spec canonical block only after its behavior/tests ship; update page dates to merge date and replace—not duplicate—superseded refresh/sid/scheduler decisions.
- [ ] Fold `Session`, `RetiredRefreshToken`, `TokenResponse`, `AccessTokenClaims.sid`, and closed audit enum changes into canonical types; mirror logical session/retired model changes without incorrectly closing its intentionally open event-type string.
- [ ] Verify JSON syntax/schema references and prose links; check canonical pages describe the exact adapter and runtime behavior actually delivered.
- [ ] Mark source spec Merged with merge date, move it to `changes/merged/`, and update its Change specs row; retain this plan in Plans as Planned/Done according to actual completion.
- [ ] Confirm no done-certificate files exist for this plan and record the intentional omission in final task/plan evidence.

## Definition of done

- [ ] Every affected canonical target named by the source spec is updated exactly once or explicitly proven unchanged by the source (for example logical-model open `event_type`).
- [ ] JSON parses, all relative links resolve, no superseded reusable/hash-sid decision remains, and no aspirational change leaks into canonical text before implementation.
- [ ] Source-spec status/location and `.specs/README.md` indexes agree.
- [ ] No `*-certificate.md` is created anywhere in this plan directory.
- [ ] Done certificates remain intentionally absent.
