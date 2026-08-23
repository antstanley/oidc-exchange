# Task 01 — Reproducible Node and Lambda inputs

**Plan:** [plan.md](../plan.md)

**Implements:** [source spec](../../../changes/2026-08-05-harden_release_supply_chain.md) §Proposed changes → Release pipeline, Node.js paragraph and Supply-chain gates → Pinning/Lockfiles; §Implementation notes A.3–A.5; §Regression tests; [distribution canonical page](../../../bindings/specs/05-distribution.md) §Release pipeline
**Depends on:** —
**Produces:** committed Node and Lambda lockfiles support frozen install commands in CI and tagged builds.
**Pointers:** `bindings/nodejs/package.json:30-36`; `bindings/nodejs/pnpm-lock.yaml`; `bindings/lambda/package.json:26-35`; `pnpm-workspace.yaml:1-42`; `.github/workflows/release.yml:299-325,438-454`; `.github/workflows/ci.yml:47-84`; `.github/workflows/nodejs-addon-glibc.yml:47-55`

## Steps

- [ ] Regenerate `bindings/nodejs/pnpm-lock.yaml` from the declared `@napi-rs/cli` major and add a committed `bindings/lambda/pnpm-lock.yaml` that resolves its workspace dependency reproducibly.
- [ ] Set `minimumReleaseAge` explicitly in `pnpm-workspace.yaml` and preserve the first-party `@oidc-exchange/*` exclusion required for self-referential platform packages.
- [ ] Replace the named Node and Lambda workflow installs with `pnpm install --frozen-lockfile`, including the release Node build, Lambda staging path, CI binding checks, and glibc-floor workflow.
- [ ] Add a focused workflow/lockfile regression harness that proves frozen installs succeed for both packages and rejects lockfile/package-manifest drift without resolving a new graph.

## Definition of done

- [ ] Node and Lambda each have an in-scope committed lockfile, and each listed CI/release install is frozen.
- [ ] Positive tests run `pnpm install --frozen-lockfile` for both packages; negative tests demonstrate stale manifest/lockfile input is rejected.
- [ ] The release-age setting and first-party exclusion are explicit and tested or statically asserted.
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: a reviewer can inspect the committed lockfiles and run the two frozen installs without a dependency rewrite.

## Sibling boundaries

- Do not change the fail-open behavior when checksum tools are absent; that installer branch belongs to the fail-closed sibling change.
