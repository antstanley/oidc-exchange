# Task 06 — sync canonical spec pages to shipped behaviour

**Plan:** [plan.md](../plan.md) · **Certificate:** [06-spec_page_sync-certificate.md](06-spec_page_sync-certificate.md)

**Implements:** [03-service-flows.md](../../../service/specs/03-service-flows.md) §Revocation, [04-http-api.md](../../../service/specs/04-http-api.md) §Routes / §Middleware stack / §Bootstrap / §Error mapping / §Decisions, [06-configuration.md](../../../service/specs/06-configuration.md) §`[server]` / §Defaults summary, and [development-guidelines.md](../../../development-guidelines.md) §"Errors are data, not exceptions" / §"Guidelines for AI agents" rule 3
**Depends on:** 01, 02, 03, 04, 05 (review — the pages document behaviour only after it is shipped, and are verified true against the code)
**Produces:** the canonical spec pages and the dev-guidelines carve-out read true against the shipped code — every "Affected spec page" from the change spec is reconciled.
**Pointers:** the "Proposed changes" blocks in `.specs/changes/2026-07-01-server_error_handling_and_shutdown.md`; the five pages named above.

## Steps

- [x] Apply the Revocation rewrite to `03-service-flows.md` (token-state failures still 200; session-repo/backend failures propagate to 503) and bump its `**Date:**`.
- [x] Apply to `04-http-api.md`: the `/revoke` route row; the rewritten Request-ID middleware entry 1 plus the inserted Request-timeout entry 2 (renumbering audit-context and catch-panic to 3 and 4); the Bootstrap graceful-shutdown step 6; the Error-mapping "log the internal detail … carries the request id" sentence; and the _200 for token state, 503 for infrastructure_ Decision. Bump its `**Date:**`.
- [x] Apply to `06-configuration.md`: add the `request_timeout` key to the `[server]` section and the `request_timeout = "30s"` entry to the Defaults-summary server row. Bump its `**Date:**`.
- [x] Narrow the `development-guidelines.md` carve-out — the "two documented best-effort paths" wording under _Errors are data, not exceptions_ and AI-agent rule 3 — to token-verification failures only (backend/session-repo failures on `/revoke` propagate and map to 503). Bump its `**Date:**`.
- [x] Verify `07-telemetry-and-audit.md` needs no text change — its request-id correlation claim now reads true given task 01 (confirm, do not edit).
- [x] Cross-check each edited page against the shipped code (tasks 01–05) so no sentence overstates or understates the behaviour; leave the change-spec status flip, the move to `changes/merged/`, and the `README.md` index to the orchestrator (do not touch them).

## Definition of done

- [x] Every "Proposed changes" block in the change spec is applied to its canonical page, and each edited page's `**Date:**` is bumped.
- [x] The dev-guidelines carve-out (error-swallowing rule and AI-agent rule 3) names token-verification failures only, with backend/session-repo `/revoke` failures explicitly propagating to 503.
- [x] `07-telemetry-and-audit.md` is confirmed accurate without edit; `canonical-types.schema.json` is untouched (no type change).
- [x] Each edited claim matches the shipped code from tasks 01–05 (no stale or aspirational sentence remains); the change-spec merge steps (status flip, move, README) are left to the orchestrator.
- [x] Meets the repo definition of done applicable to docs (prose accurate, internal links resolve — see plan.md baseline).
- [x] Reviewable: a reviewer reads the four edited pages against the diff of tasks 01–05 and confirms each sentence is true and no affected page is left stale.
