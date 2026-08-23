# Task 07 — Canonical spec and docs sync

**Plan:** [plan.md](../plan.md)

**Implements:** [source spec](../../../changes/merged/2026-08-05-harden_release_supply_chain.md) §Affected spec pages; §Proposed changes → Artifacts, Install script, Release pipeline, Supply-chain gates, Assumptions / Decisions, and Repository hygiene; §Merge plan; [distribution canonical page](../../../bindings/specs/05-distribution.md) §§Artifacts, Install script, Release pipeline; [development guidelines](../../../development-guidelines.md) §Repository hygiene
**Depends on:** 02, 04, 05, 06
**Produces:** canonical distribution/hygiene rules and public installation instructions accurately describe the implemented supply-chain controls.
**Pointers:** `.specs/bindings/specs/05-distribution.md:8-98`; `.specs/development-guidelines.md:290-304`; `README.md:11-23`; `docs/getting-started/quick-start.md:13-25`; `.github/workflows/release.yml:578-618`; `.gitignore:105-108`; `examples/linux-sqlite/setup.sh:9-16`; `.specs/README.md:28-80`

## Steps

- [x] Apply the source spec’s distribution-page updates: provenance column, installer validation/verification behavior, per-job permissions and frozen/pinned release rules, Supply-chain gates, and closing-block additions.
- [x] Apply the repository-hygiene canonical updates for advisory enforcement and pattern-based generated key/local-state ignores; add the listed `.gitignore` patterns and verify no tracked object is hidden unexpectedly.
- [x] Update README, quick-start, and generated release-body installation text to lead with the attestation-verifying path and document image verification beside Docker pull.
- [x] Perform merge housekeeping only after all task controls land: update canonical page dates/status as required, move/mark the change spec according to its merge plan, and update `.specs/README.md`’s change-spec index.
- [x] Add documentation/link checks covering canonical headings, release snippets, and the published command examples; retain the explicit sibling boundary in installer-related prose.

## Definition of done

- [x] Canonical pages state the implemented provenance, lockfile, permissions, advisory, and hygiene invariants without claiming unbuilt controls.
- [x] Public binary and container instructions show a concrete verifying path; links and command snippets resolve from their authored locations.
- [x] `.gitignore` uses the required patterns (`*.pem`, `*.p8`, `*.key`, `keys/`, `data/`, `lmdb/`, `*.db`, `*.sqlite`, `*.sqlite3`) and tracked-file checks show no accidental masking.
- [x] Change-spec merge/index updates occur only when all in-scope packages are merged; the sibling fail-closed behavior remains separately owned.
- [x] Documentation and static link checks cover positive references and a broken-link negative fixture where supported.
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: a reviewer can follow the documented binary/container verification instructions and trace each canonical rule to an implemented workflow or installer control.

## Sibling boundaries

- Do not update canonical prose to claim the installer fails closed when no checksum utility exists; that promise and its implementation stay with the unstacked fail-closed sibling.

## Final implementation audit (2026-08-24)

- Canonical distribution and repository-hygiene pages now match the shipped binary, per-platform
  image, final-manifest, installer, least-privilege, frozen-input, advisory, and signing-path controls.
- Public commands fix repository `antstanley/oidc-exchange` and signer workflow
  `antstanley/oidc-exchange/.github/workflows/release.yml`; GHCR verification is not overclaimed for
  Docker Hub, and build provenance is explicitly distinguished from registry signing.
- Advisory inventory is 18 exact active exceptions: Cargo 7, pnpm 11, Python 0. Cargo additionally
  reports two warning-only records (unmaintained `bincode 1.3.3`, yanked `spin 0.9.8`). The pyo3
  exceptions are absent after the 0.29.2 migration. Signing-path policy has 14 exact temporary RC
  exceptions: seven in each of two actual metadata modes, all expiring 2026-09-15.
- Exact pip-audit 2.9.0 was provisioned with `uv pip install --require-hashes --only-binary=:all:`
  from `config/pip-audit-requirements.txt`. The full live wrapper executed Cargo, pnpm, and Python.
  Python's frozen build export is nonempty and contains maturin 1.9.4 plus conditional tomli 2.4.1; the production runtime export remains separately empty because the abi3 extension has no runtime Python dependencies; the wrapper uses pip-audit's no-resolver mode so this clean graph is evaluated
  without an unrelated nested-venv/ensurepip failure.
- No publication, registry write, GitHub merge, bookmark move, history rewrite, secret, token, or
  certificate was created. The sibling missing-checksum-tool behavior remains unchanged.

## Gate evidence

- pnpm 11.9.0 lockfile-only/frozen/ignore-scripts: 3 owned entry points passed.
- Supply-chain policy/negative fixtures: 42/42 passed; installer: 15/15 passed.
- Rust: fmt and clippy passed; nextest 387/387 passed (27 skipped by suite configuration).
- Node binding: build, lint, typecheck, 6/6 tests passed. Lambda: build, lint, typecheck, 11/11 tests passed.
- Python: maturin develop/import and release `cp310-abi3` wheel passed; pytest 13/13, Ruff format/check, and Pyright passed.
- Live advisory wrapper: Cargo 0 allowed/2 warning/0 failure; pnpm 11 allowed/0 warning/0 failure; Python 0/0/0.
- Live signing path: both modes passed with 6 exercised prerelease paths each and 0 failures.
- Workflow YAML/policy JSON, full action-SHA/permissions/attestation handoff static tests, official v3 peeled ref SHA verification (`977bb373ede98d70efdf65b84cb5f73e068dcc2a`), Bash syntax, shellcheck, and canonical/public command assertions passed. shfmt was unavailable. Repo-level pnpm scripts that trigger `prepare` were not used under jj; direct package commands passed as required.

## Review-round-1 remediation evidence

- Canonical and public docs now describe the audited Python build graph, seven signing findings per mode, exact cross 0.2.5 provisioning, and truthful checksum/provenance states. They explicitly preserve the sibling-owned warn-and-continue behavior when both checksum tools are missing and do not claim fail-closed checksum handling.
