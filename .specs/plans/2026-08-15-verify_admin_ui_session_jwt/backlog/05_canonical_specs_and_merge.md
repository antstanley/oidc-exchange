# Task 05 — Canonical specs and merge

**Plan:** [plan.md](../plan.md) · **Certificate:** Intentionally omitted at user direction; do not create a certificate file.

**Implements:** [change spec §Affected spec pages](../../../changes/2026-08-05-verify_admin_ui_session_jwt.md#affected-spec-pages), [§Proposed changes](../../../changes/2026-08-05-verify_admin_ui_session_jwt.md#proposed-changes), and [§Merge plan](../../../changes/2026-08-05-verify_admin_ui_session_jwt.md#merge-plan)
**Depends on:** 02, 03, 04
**Produces:** canonical admin-UI and development-guidelines pages accurately document the merged verified-session behavior, while change-spec and README bookkeeping make the completed change discoverable.
**Pointers:** `.specs/admin-ui/specs/00-overview.md:25-87`; `.specs/development-guidelines.md:330-369`; `.specs/README.md:28-80`; `.specs/changes/2026-08-05-verify_admin_ui_session_jwt.md:52-62`; `.specs/changes/2026-08-05-harden_admin_plane.md`; `.specs/changes/2026-08-05-fail_closed_across_config_and_adapters.md`

## Steps

- [ ] Apply the Authentication model, Environment, Assumptions, Decisions, and Open questions deltas to the canonical Admin UI overview, update its date, and retain links to the service discovery/JWKS contract.
- [ ] Extend the Development Guidelines `Three toolchains, one CI` decision to state that the web-apps job runs the admin UI's `pnpm test`, then update its date.
- [ ] After implementation and scoped verification are complete, mark this change spec merged, add the merge date, move it to `changes/merged/`, and update the Change specs table in `.specs/README.md`.
- [ ] Re-check all canonical and change-spec links after the move, preserve the plan's README entry, and do not merge or modify sibling proposed specs.
- [ ] Record the actual scoped TypeScript gate results and separately report, without remediation, the known main `cargo test --workspace` baseline failure of three config tests caused by missing `providers.*.adapter`.

## Definition of done

- [ ] Canonical Admin UI documentation says only verified claims reach the gate and login paths, names the configured issuer/audience requirements, exact admin claim comparison, host-prefixed strict cookie, and fail-closed behavior.
- [ ] Canonical development guidance says CI executes the admin UI test suite, and page dates/change-spec/README status accurately reflect the merge state.
- [ ] All moved-document and canonical links resolve, and this PR remains scoped to its change spec rather than folding in the audience-config or admin-plane sibling changes.
- [ ] Scoped TypeScript gates pass; any `cargo test --workspace` result is reported with the known three unrelated missing-`providers.*.adapter` config-test failures left untouched.
- [ ] Meets the repo definition of done (named bounds, assertions, TypeScript tests, `pnpm format:check`, `pnpm lint`, `pnpm typecheck`, and `pnpm test`; see plan.md baseline).
- [ ] Reviewable: compare the merged canonical pages and README table with the implemented verifier and CI job, then follow every link from the moved change spec.
