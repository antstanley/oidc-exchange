# Task 04 — CI test gate

**Plan:** [plan.md](../plan.md) · **Certificate:** Intentionally omitted at user direction; do not create a certificate file.

**Implements:** [change spec §Proposed changes — Development Guidelines](../../../changes/2026-08-05-verify_admin_ui_session_jwt.md#specsdevelopment-guidelinesmd--decisions-three-toolchains-one-ci-modify) and [§Implementation notes 8](../../../changes/2026-08-05-verify_admin_ui_session_jwt.md#implementation-notes)
**Depends on:** 03
**Produces:** the existing `web-apps` workflow invokes `pnpm test` in `apps/admin-ui` after its lint, format, and typecheck gates.
**Pointers:** `.github/workflows/ci.yml:121-147`; `apps/admin-ui/package.json:6-17`; `.specs/development-guidelines.md:330-344`

## Steps

- [ ] Add an `Admin UI — test` workflow step in the existing `web-apps` job using `apps/admin-ui` as its working directory and the package test script from Task 03.
- [ ] Keep the job's dependency installation and existing website/admin lint, format, and typecheck ordering intact so the added test gate uses the same locked pnpm dependency graph.
- [ ] Run the scoped local command sequence and verify the workflow YAML and package script resolve without changing unrelated CI jobs.

## Definition of done

- [ ] The `web-apps` job runs `pnpm test` for `apps/admin-ui` in addition to existing lint, format-check, and typecheck commands.
- [ ] The workflow invokes the same focused test script developers run locally and does not add an alternate or skipped security-test path.
- [ ] Scoped TypeScript format, lint, typecheck, and test commands pass locally before the task is considered complete.
- [ ] Meets the repo definition of done (named bounds, assertions, TypeScript tests, `pnpm format:check`, `pnpm lint`, `pnpm typecheck`, and `pnpm test`; see plan.md baseline).
- [ ] Reviewable: read the `web-apps` job and run its admin-UI command sequence to see the JWT regression suite included in the CI gate.
