# Done Certificate — Task 06: sync canonical spec pages to shipped behaviour

**Task:** [06-spec_page_sync.md](06-spec_page_sync.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified

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
  - *Status:* ☐ unverified

- **O2 — Dev-guidelines carve-out narrowed to token-verification failures.**
  - *Claim:* the "two documented best-effort paths" wording and AI-agent rule 3 in
    `development-guidelines.md` now name token-verification failures only, with backend/session-repo
    `/revoke` failures propagating to 503.
  - *Evidence to collect:* read `development-guidelines.md` §"Errors are data, not exceptions" and
    §"Guidelines for AI agents" rule 3; confirm the narrowed wording and the bumped `**Date:**`.
  - *Status:* ☐ unverified

- **O3 — `07-telemetry-and-audit.md` confirmed accurate without edit; schema untouched.**
  - *Claim:* the telemetry page's request-id correlation claim reads true given task 01 and needs
    no edit; `canonical-types.schema.json` is unchanged.
  - *Evidence to collect:* read the correlation sentence in `07-telemetry-and-audit.md` and confirm
    it is accurate; confirm no diff to `canonical-types.schema.json`.
  - *Status:* ☐ unverified

- **O4 — Each edited claim matches the shipped code (tasks 01–05); merge steps left to orchestrator.**
  - *Claim:* no edited sentence overstates or understates the shipped behaviour, and the status
    flip / move / README are not done here.
  - *Evidence to collect:* cross-read each edited claim against the diffs of tasks 01–05 (e.g. the
    503 path, the 408 timeout, the 10 s drain, the request span); confirm the change spec's
    `Status` is still `Proposed`/unflipped and `README.md` is untouched by this task.
  - *Status:* ☐ unverified

- **O5 — Meets the repo definition of done applicable to docs.**
  - *Claim:* the edited prose is accurate and every internal link in the edited pages resolves
    (the doc-applicable subset of the repo DoD — no test/lint gate, as this task has no runtime
    surface).
  - *Evidence to collect:* re-read the edited prose for accuracy against tasks 01–05; check that
    every relative link in the edited pages points at an existing file.
  - *Status:* ☐ unverified

- **O6 — Reviewable: the four edited pages read true against the code, none left stale.**
  - *Claim:* a reviewer reads the four edited pages against the tasks 01–05 diff and confirms each
    sentence is true and no affected page is stale.
  - *Evidence to collect:* walk the "Affected spec pages" table from the change spec and confirm
    each row is reconciled in the edited pages.
  - *Status:* ☐ unverified

## Regression check

- Internal links in the edited pages (cross-references between 03/04/06 and to
  `development-guidelines.md`) still resolve after the edits : ☐ (PRESERVED / REGRESSION)

## Residue

- This is a documentation task with no runtime surface; its correctness is a prose-vs-code match,
  so the repo DoD reduces to accurate prose and resolving links (no test/lint gate beyond that).

## Conclusion

VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
