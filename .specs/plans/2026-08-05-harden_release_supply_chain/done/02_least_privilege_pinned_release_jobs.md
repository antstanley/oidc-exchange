# Task 02 — Least-privilege pinned release jobs

**Plan:** [plan.md](../plan.md)

**Implements:** [source spec](../../../changes/merged/2026-08-05-harden_release_supply_chain.md) §Proposed changes → Release pipeline and Node.js paragraph; §Supply-chain gates → Pinning/Least privilege; §Implementation notes A.2–A.3; [distribution canonical page](../../../bindings/specs/05-distribution.md) §Release pipeline
**Depends on:** 01
**Produces:** per-job permissions and locked/exact tooling prevent jobs with publishing authority from executing dynamically resolved packages.
**Pointers:** `.github/workflows/release.yml:8-10,16-618`; `.github/workflows/ci.yml:12-148`; `.github/workflows/nodejs-addon-glibc.yml:21-79`; `bindings/nodejs/package.json:22-36`

## Steps

- [ ] Remove workflow-level permissions and assign every release, CI, and glibc-floor job only the scopes it requires; retain `contents: read` where checkout runs and set `persist-credentials: false` for non-pushing checkout jobs.
- [ ] Move npm package validation into `validate-npm-package` with read-only permissions, add it to `publish-npm` needs, and keep publishing credentials isolated from validation tooling.
- [ ] Replace global/unpinned napi CLI, npm upgrade, `npx --yes publint`, and `npx --yes @arethetypeswrong/cli` resolution with exact versions or workspace-locked dev dependencies; remove the npm upgrade if bundled npm satisfies staged publishing.
- [ ] Add static workflow tests that derive write/publishing jobs from their `permissions:` blocks and reject dynamic fetch patterns, insufficient checkout scope, persisted credentials in non-pushing jobs, or a publish path that bypasses validation.

## Definition of done

- [ ] No workflow-level permission grant remains; every job has an explicit least-privilege block consistent with its actions.
- [ ] A job with `id-token: write`, `contents: write`, `packages: write`, or attestation authority contains no `@latest`, `npx --yes`, unversioned global add, or non-frozen install.
- [ ] Package validation runs separately with `contents: read`, and `publish-npm` cannot start without it and Node artifacts.
- [ ] Positive and negative workflow-invariant tests cover existing and injected violating examples without publishing.
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: a reviewer can trace each job’s permissions and prove dynamic package resolution cannot run beside publishing authority.

## Sibling boundaries

- Do not implement missing-checksum-tool failure behavior from the fail-closed sibling; only release workflow and package-resolution controls are owned here.

## Review-round-1 remediation evidence

- All release `cargo install cross` sites use literal stable `cross 0.2.5` with `--locked`; workflow policy accepts that exact form and rejects missing, variable, range, and prerelease versions. Crates.io metadata (`cargo search cross`) reported 0.2.5 as the current stable release. Job permissions and pinned attestation actions remain unchanged, so attested bytes are still produced by the reviewed tool.
