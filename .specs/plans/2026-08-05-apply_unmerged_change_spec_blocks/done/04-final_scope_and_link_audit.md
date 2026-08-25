# 04 · Final scope and link audit

**Plan:** [plan.md](../plan.md) · **Status:** Done · **Certificate:** intentionally omitted

**Implements:** PR #29 as bounded by `spec/apply-unmerged-change-spec-blocks`; reviews the source coverage in tasks 01–03.
**Depends on:** 01 · configuration_and_ffi; 02 · lifecycle_and_admin_canonical_pages; 03 · audit_and_lambda_http_canonical_pages
**Produces:** a verified PR-ready documentation diff limited to the six canonical pages.

## Steps

- [x] Compare the unstacked branch diff with the four source specs' `Affected spec pages`, `Proposed changes`, and `Merge plan` sections.
- [x] Confirm each changed canonical page has date `2026-08-05`, remains `Implemented`, and has no stale resolved open question from the source blocks.
- [x] Validate relative Markdown links in this plan and all task packages; validate that task dependencies reference existing lower-numbered tasks.
- [x] Confirm the kanban contains exactly four done tasks, no backlog/in-progress/blocked tasks, and no done certificates; confirm the active plan index remains `spec/apply-unmerged-change-spec-blocks` at `@-`.
- [x] Confirm changed paths are exactly the six canonical pages plus this plan and its four task packages; reject scope expansion.

## Definition of done

- [x] Source-block coverage is complete: 8 complete-config blocks, 6 lifecycle blocks, 10 audit blocks, and 2 Lambda blocks are accounted for by tasks 01–03.
- [x] DAG is acyclic and valid: 01–03 have no dependencies; 04 depends only on 01–03; every edge targets a lower number.
- [x] Task status is internally consistent: the plan and all four task packages are `Done`; no certificates were created.
- [x] Link validation, content review, and path-scope validation pass; no implementation test command is claimed for this documentation-only planning change.

## Execution evidence

- Independent final gate: correctness PASS; Definition-of-Done completeness PASS.
- Integration validated all local Markdown targets across the six canonical pages, plan, and four task packages. The initial PR diff was bounded to those files; execution only moved the task packages to `done/` and recorded completion.
