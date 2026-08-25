# Task 09 — Integration, regression, and handoff

**Plan:** [plan.md](../plan.md)  
**Status:** Done
**Implements:** [source spec](../../../changes/merged/2026-08-05-fail_closed_across_config_and_adapters.md) → all implementation notes, compatibility, merge plan, assumptions, and decisions  
**Depends on:** 08  
**Produces:** a final evidence report that verifies end-to-end fail-closed behavior, documents known baseline-red config/adapters failures without repairing unrelated work, validates plan/canonical links and source coverage, and hands off sibling dependencies.  
**Pointers:** [plan](../plan.md); all Task 01–08 packages; source spec; `.specs/development-guidelines.md`; CI/repository test commands.

## Steps

- [x] Run targeted tests for resolver/entrypoints, exchange policy, local/KMS algorithms,
  provider/discovery HTTPS behavior, PostgreSQL probe, installer harness, schemas, and examples;
  report commands and exact pass/fail output summaries.
- [x] Run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, and
  `cargo nextest run --workspace` where environment permits. The planning baseline run on this
  worktree was green (387 passed, 27 skipped) despite the brief's reported red baseline; if a
  future run is red, separately classify pre-existing config/adapters failures from regressions
  introduced by this PR and do not fix unrelated failures.
- [x] Verify the closed-domain corpus covers every source-spec field/domain and every stated
  compatibility break; verify no production path contains an implicit permissive fallback.
- [x] Verify all Markdown links in this plan package and canonical/source links; verify the
  task-table dependency graph matches every task header and remains acyclic.
- [x] Verify task status/kanban placement matches plan status, all DoDs are checkable, no
  certificate files/links were created, and source-spec merge/index steps are deferred until
  actual completion.
- [x] Produce concise handoff notes naming the placeholder, admin-plane, runtime-parity,
  release-supply-chain, and audit/throttle sibling boundaries and merge order.

## Definition of done

- [x] Targeted tests demonstrate each decisive denial/failure and each intended valid path.
- [x] Required repository commands have actual results recorded; pre-existing red failures include
  reproducible commands and are explicitly excluded from this PR's fix scope.
- [x] Link, coverage, DAG, DoD, status, canonical, and no-certificate audits all pass.
- [x] No commit or push is performed as part of this planning/verification task.

## Execution evidence — 2026-08-16

- **Passed:** `cargo fmt --all --check`; `cargo clippy --workspace -- -D warnings`; and
  `bash scripts/test-install.sh` (the hermetic verifier-missing, supported-verifier, and bad
  checksum cases passed). The core closed-domain suite ran **6/6 passed**: accepted domains plus
  invalid fields, HTTP URL rejection, and local/KMS algorithm rejection.
- **Final workspace verification:** `cargo nextest run --workspace --no-fail-fast` completed
  green with **389 passed, 27 skipped**. This supersedes the earlier disk-space interruption; no
  test assertion failure or baseline-red classification remains.
- **Static/integration audit:** implementation uses resolved closed types at config consumers,
  exhaustive registration-mode handling, HTTPS-only production URLs (test-only Wiremock seam),
  discovery `is_success`, the Postgres index/version probe, and mandatory installer verifier.
  KMS example vocabulary is absent outside rejection tests; configured issuer/provider/webhook
  TOML/YAML examples contain no `http://`; service `AccessTokenClaims.iss`/`.aud` both have
  `minLength: 1`; source spec is merged and indexed; all eight named canonical pages are updated.
  The dependency table matches task headers and is acyclic; this plan package has no certificates.
- **Board audit:** source-spec links now target `changes/merged/`; task statuses and kanban
  placement are Done, all Steps/DoD boxes are checked, and this package contains no certificates.

## Handoff

- Placeholder resolution remains downstream of the shared `resolve()` seam; do not absorb it.
  Admin-plane placement, runtime parity, and release-supply-chain `--version`/attestation remain
  sibling-owned. Audit/throttle merges after this change and supersedes this change's `audit.noop`
  snapshot; neither its durability/rate-limit keys nor `stdout` default is included here.

## Sibling boundaries

- This is a handoff task, not an implementation sink: it records dependencies and does not take
  sibling changes merely to make the final report look green.
