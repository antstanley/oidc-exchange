# Done Certificate — Task 06: sync canonical spec pages to shipped behaviour

**Task:** [06-spec_page_sync.md](06-spec_page_sync.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

> Verification protocol for Task 06. A validating agent discharges it: for each obligation,
> collect the named evidence, run the named checks, set the Status, then derive the Conclusion
> by the rubric. Do not mark an obligation SATISFIED without its evidence; do not record DONE
> with any non-SATISFIED obligation.

## Definition

DONE(Task 06) ≡ every obligation O1…O6 below holds, each backed by the evidence it names — not
by assertion.

## Premises

- **P1 — Goal.** The canonical spec pages (03/04/06) and the dev-guidelines carve-out read true
  against the shipped code — every "Affected spec page" from the change spec is reconciled.
- **P2 — Obligations.** Done iff O1…O6 all hold, in DoD order; O6 is the Reviewable item.
- **P3 — Invariants.** Must not flip the change-spec `Status`, move it to `changes/merged/`, or
  edit `.specs/README.md` (orchestrator's), and must not touch `canonical-types.schema.json`.

## Obligations

- **O1 — Every proposed-changes block applied; each edited page's `Date` bumped.**
  - *Claim:* the Revocation rewrite (03), the `/revoke` row + middleware entries 1–2 + Bootstrap
    step 6 + Error-mapping sentence + Decision (04), and the `[server]` `request_timeout` key +
    Defaults-summary row (06) are all applied, with each page's `**Date:**` bumped.
  - *Evidence to collect:* diff `03-service-flows.md`, `04-http-api.md`, `06-configuration.md`
    against the change spec's "Proposed changes" blocks; confirm each block is present and each
    `**Date:**` changed.
  - *Checks:* confirm the middleware renumber — audit-context and catch-panic became entries 3 and
    4 after the inserted request-timeout entry 2.
  - *Status:* SATISFIED — all three proposed-changes blocks are applied verbatim. `03-service-flows.md:57-69`
    carries the Revocation rewrite (token-state failures still 200; `revoke_all_user_sessions` /
    `revoke_session` / lookup errors propagate to 503). `04-http-api.md` has the `/revoke` row
    (`:16` "200 for invalid/unknown tokens, 503 on backend failure"), the rewritten Request-ID
    entry 1 with the per-request `info_span` and inserted Request-timeout entry 2 (`:65-71`),
    Bootstrap graceful-shutdown step 6 (`:105-110`), the Error-mapping request-id sentence
    (`:133-135`), and the Decision (`:152-155`). `06-configuration.md` has the `request_timeout`
    `[server]` key (`:48-50`) and the Defaults-summary row (`:101`). Each page's `**Date:**` is
    bumped to `2026-07-02` (03 and 04 were `2026-06-24`, dev-guidelines/06 shown bumped in diff).
    Check: middleware renumber confirmed — Audit-context is entry 3 (`:72`) and Catch-panic entry 4
    (`:74`) after the inserted Request-timeout entry 2, matching `bootstrap.rs`'s last-`.layer()`-outermost
    ordering (request_id outermost, TimeoutLayer, audit_context, CatchPanic innermost).

- **O2 — Dev-guidelines carve-out narrowed to token-verification failures.**
  - *Claim:* the "two documented best-effort paths" wording and AI-agent rule 3 in
    `development-guidelines.md` now name token-verification failures only, with backend/session-repo
    `/revoke` failures propagating to 503.
  - *Evidence to collect:* read `development-guidelines.md` §"Errors are data, not exceptions" and
    §"Guidelines for AI agents" rule 3; confirm the narrowed wording and the bumped `**Date:**`.
  - *Status:* SATISFIED — `development-guidelines.md:112-115` narrows the carve-out to "user-sync
    notifications, and token-verification failures in the RFC 7009 revoke response — backend and
    session-repo failures on `/revoke` propagate and map to 503". Rule 3 at `:313-315` reads
    "the two documented ones (user-sync, and revoke's token-verification failures — never revoke's
    backend errors)" — verified against the file directly (the "revokeand" seen in the word-level
    diff is a display artifact; the rendered text is clean). `**Date:**` bumped `2026-06-29` →
    `2026-07-02`. Both match the change spec's proposed blocks verbatim.

- **O3 — `07-telemetry-and-audit.md` confirmed accurate without edit; schema untouched.**
  - *Claim:* the telemetry page's request-id correlation claim reads true given task 01 and needs
    no edit; `canonical-types.schema.json` is unchanged.
  - *Evidence to collect:* read the correlation sentence in `07-telemetry-and-audit.md` and confirm
    it is accurate; confirm no diff to `canonical-types.schema.json`.
  - *Status:* SATISFIED — `07-telemetry-and-audit.md:41-42` ("A single request can produce both a
    telemetry trace and an audit event; they correlate through the request id but are otherwise
    independent") reads true now that `middleware/request_id.rs:50-52` opens a per-request
    `info_span!("request", request_id = %request_id, …)` so downstream logs inherit the field. The
    page is unedited — its `**Date:**` is still `2026-06-24` and it is not in the `jj diff` — as
    required. `canonical-types.schema.json` is absent from `jj status`/`jj diff` (untouched).

- **O4 — Each edited claim matches the shipped code (tasks 01–05); merge steps left to orchestrator.**
  - *Claim:* no edited sentence overstates or understates the shipped behaviour, and the status
    flip / move / README are not done here.
  - *Evidence to collect:* cross-read each edited claim against the diffs of tasks 01–05 (e.g. the
    503 path, the 408 timeout, the 10 s drain, the request span); confirm the change spec's
    `Status` is still `Proposed`/unflipped and `README.md` is untouched by this task.
  - *Status:* SATISFIED — every edited claim matches the shipped code. The 503 revoke path:
    `crates/core/src/service/revoke.rs:99-102` propagates a lookup `Err` via `?` (the prior
    `.ok().flatten()` swallow is gone), `:114` propagates `revoke_session`, `:53` propagates
    `revoke_all_user_sessions`; the working copy also adds the tests
    `revoke_lookup_failure_returns_503_refresh_token` and
    `revoke_unknown_refresh_token_lookup_ok_none_returns_200` (`crates/server/tests/routes.rs:241-305`)
    plus the `session_lookup_fail_mode` harness — all green. The 408 timeout:
    `bootstrap.rs:340` builds `TimeoutLayer::with_status_code(408, …)` as entry 2. The 10 s drain:
    `shutdown.rs:20 SHUTDOWN_DRAIN_DEADLINE_SECS = 10`, wired via `main.rs:54` /
    `with_graceful_shutdown`. The request span: `request_id.rs:50` `info_span!("request", …)`.
    The error-detail log: `error.rs:74` `tracing::error!(error = %err, status = %status, …)` for
    the `server_error` class, emitted inside the request-id span. Merge steps left to the
    orchestrator: the change spec `**Status:**` is still `Proposed` (`:3`) and `README.md` is
    absent from `jj diff` (untouched).

- **O5 — Meets the repo definition of done applicable to docs.**
  - *Claim:* the edited prose is accurate and every internal link in the edited pages resolves
    (the doc-applicable subset of the repo DoD — no test/lint gate, as this task has no runtime
    surface).
  - *Evidence to collect:* re-read the edited prose for accuracy against tasks 01–05; check that
    every relative link in the edited pages points at an existing file.
  - *Status:* SATISFIED — the edited prose is accurate (see O4) and no edit introduced a new
    relative link; every pre-existing relative link in the edited pages resolves: `01-…`–`07-…`
    in `service/specs/` all exist, and `development-guidelines.md`'s `service/specs/04-http-api.md`
    target exists. `cargo nextest run --workspace` → 368 passed, 0 failed, 27 skipped, confirming
    the code the prose describes builds and behaves as documented. This is a docs task with no
    additional runtime surface, so the repo DoD reduces to accurate prose + resolving links, both met.

- **O6 — Reviewable: the four edited pages read true against the code, none left stale.**
  - *Claim:* a reviewer reads the four edited pages against the tasks 01–05 diff and confirms each
    sentence is true and no affected page is stale.
  - *Evidence to collect:* walk the "Affected spec pages" table from the change spec and confirm
    each row is reconciled in the edited pages.
  - *Status:* SATISFIED — walked all five rows of the change spec's "Affected spec pages" table:
    (1) `03-service-flows.md` Revocation reconciled; (2) `04-http-api.md` `/revoke` row + Decision +
    request-id/timeout middleware + Bootstrap shutdown + error-mapping sentence all reconciled;
    (3) `06-configuration.md` `[server]` key + Defaults row reconciled; (4) `07-telemetry-and-audit.md`
    confirmed true without edit; (5) `development-guidelines.md` carve-out + rule 3 narrowed. No
    affected page is left stale, and each edited sentence reads true against the tasks 01–05 code.

## Regression check

- Internal links in the edited pages (cross-references between 03/04/06 and to
  `development-guidelines.md`) still resolve after the edits : PRESERVED — no link line was
  modified; all `01-…`–`07-…` targets in `service/specs/` exist and the dev-guidelines
  `service/specs/04-http-api.md` target exists. No existing behaviour was regressed; the only
  code change (revoke.rs lookup-error propagation) strengthens the very 503 path the docs now
  describe, and the full workspace suite (368 tests) stays green.

## Residue

- This is a documentation task with no runtime surface; its correctness is a prose-vs-code match,
  so the repo DoD reduces to accurate prose and resolving links (no test/lint gate beyond that).

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O6 all SATISFIED — every change-spec "Proposed changes" block is applied to its
canonical page with each `**Date:**` bumped, the dev-guidelines carve-out and AI-agent rule 3 are
narrowed to token-verification failures (backend/session-repo `/revoke` failures propagate to 503),
`07-telemetry-and-audit.md` and `canonical-types.schema.json` are correctly untouched, and every
edited claim reads true against the shipped tasks 01–05 code (revoke.rs lookup-error propagation,
408 timeout layer, 10 s drain, per-request span, `tracing::error!` detail log) with all links
resolving, the change-spec `Status` still `Proposed`, README untouched, and the full workspace
suite green (368 passed); regression check PRESERVED.
