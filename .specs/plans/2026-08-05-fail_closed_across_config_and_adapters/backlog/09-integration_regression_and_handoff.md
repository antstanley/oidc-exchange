# Task 09 — Integration, regression, and handoff

**Plan:** [plan.md](../plan.md)  
**Status:** Backlog  
**Implements:** [source spec](../../../changes/2026-08-05-fail_closed_across_config_and_adapters.md) → all implementation notes, compatibility, merge plan, assumptions, and decisions  
**Depends on:** 08  
**Produces:** a final evidence report that verifies end-to-end fail-closed behavior, documents known baseline-red config/adapters failures without repairing unrelated work, validates plan/canonical links and source coverage, and hands off sibling dependencies.  
**Pointers:** [plan](../plan.md); all Task 01–08 packages; source spec; `.specs/development-guidelines.md`; CI/repository test commands.

## Steps

- [ ] Run targeted tests for resolver/entrypoints, exchange policy, local/KMS algorithms,
  provider/discovery HTTPS behavior, PostgreSQL probe, installer harness, schemas, and examples;
  report commands and exact pass/fail output summaries.
- [ ] Run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, and
  `cargo nextest run --workspace` where environment permits. The planning baseline run on this
  worktree was green (387 passed, 27 skipped) despite the brief's reported red baseline; if a
  future run is red, separately classify pre-existing config/adapters failures from regressions
  introduced by this PR and do not fix unrelated failures.
- [ ] Verify the closed-domain corpus covers every source-spec field/domain and every stated
  compatibility break; verify no production path contains an implicit permissive fallback.
- [ ] Verify all Markdown links in this plan package and canonical/source links; verify the
  task-table dependency graph matches every task header and remains acyclic.
- [ ] Verify task status/kanban placement matches plan status, all DoDs are checkable, no
  certificate files/links were created, and source-spec merge/index steps are deferred until
  actual completion.
- [ ] Produce concise handoff notes naming the placeholder, admin-plane, runtime-parity,
  release-supply-chain, and audit/throttle sibling boundaries and merge order.

## Definition of done

- [ ] Targeted tests demonstrate each decisive denial/failure and each intended valid path.
- [ ] Required repository commands have actual results recorded; pre-existing red failures include
  reproducible commands and are explicitly excluded from this PR's fix scope.
- [ ] Link, coverage, DAG, DoD, status, canonical, and no-certificate audits all pass.
- [ ] No commit or push is performed as part of this planning/verification task.

## Sibling boundaries

- This is a handoff task, not an implementation sink: it records dependencies and does not take
  sibling changes merely to make the final report look green.
