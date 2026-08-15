# 04 · Final scope and link audit

**Plan:** [plan.md](../plan.md) · **Status:** Backlog · **Certificate:** intentionally omitted (planning backlog only)

**Implements:** PR #29 as bounded by `spec/apply-unmerged-change-spec-blocks`; reviews the source coverage in tasks 01–03.
**Depends on:** 01 · configuration_and_ffi; 02 · lifecycle_and_admin_canonical_pages; 03 · audit_and_lambda_http_canonical_pages
**Produces:** a verified PR-ready documentation diff limited to the six canonical pages.

## Steps

- [ ] Compare the unstacked branch diff with the four source specs' `Affected spec pages`, `Proposed changes`, and `Merge plan` sections.
- [ ] Confirm each changed canonical page has date `2026-08-05`, remains `Implemented`, and has no stale resolved open question from the source blocks.
- [ ] Validate relative Markdown links in this plan and all backlog packages; validate that task dependencies reference existing lower-numbered tasks.
- [ ] Confirm the kanban contains exactly four backlog tasks, no in-progress/blocked/done tasks, and no done certificates; confirm the active plan index remains `spec/apply-unmerged-change-spec-blocks` at `@-`.
- [ ] Confirm changed paths are exactly the six canonical pages plus this plan and its four backlog files; reject scope expansion.

## Definition of done

- [ ] Source-block coverage is complete: 8 complete-config blocks, 6 lifecycle blocks, 10 audit blocks, and 2 Lambda blocks are accounted for by tasks 01–03.
- [ ] DAG is acyclic and valid: 01–03 have no dependencies; 04 depends only on 01–03; every edge targets a lower number.
- [ ] Task status is internally consistent: plan and all four task packages are `Backlog`; no execution evidence or certificate is claimed.
- [ ] Link validation, content review, and path-scope validation pass; no implementation test command is claimed for this documentation-only planning change.
