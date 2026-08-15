# Task 07 — Canonical spec and docs sync

**Plan:** [plan.md](../plan.md)

**Implements:** [source spec](../../../changes/2026-08-05-harden_release_supply_chain.md) §Affected spec pages; §Proposed changes → Artifacts, Install script, Release pipeline, Supply-chain gates, Assumptions / Decisions, and Repository hygiene; §Merge plan; [distribution canonical page](../../../bindings/specs/05-distribution.md) §§Artifacts, Install script, Release pipeline; [development guidelines](../../../development-guidelines.md) §Repository hygiene
**Depends on:** 02, 04, 05, 06
**Produces:** canonical distribution/hygiene rules and public installation instructions accurately describe the implemented supply-chain controls.
**Pointers:** `.specs/bindings/specs/05-distribution.md:8-98`; `.specs/development-guidelines.md:290-304`; `README.md:11-23`; `docs/getting-started/quick-start.md:13-25`; `.github/workflows/release.yml:578-618`; `.gitignore:105-108`; `examples/linux-sqlite/setup.sh:9-16`; `.specs/README.md:28-80`

## Steps

- [ ] Apply the source spec’s distribution-page updates: provenance column, installer validation/verification behavior, per-job permissions and frozen/pinned release rules, Supply-chain gates, and closing-block additions.
- [ ] Apply the repository-hygiene canonical updates for advisory enforcement and pattern-based generated key/local-state ignores; add the listed `.gitignore` patterns and verify no tracked object is hidden unexpectedly.
- [ ] Update README, quick-start, and generated release-body installation text to lead with the attestation-verifying path and document image verification beside Docker pull.
- [ ] Perform merge housekeeping only after all task controls land: update canonical page dates/status as required, move/mark the change spec according to its merge plan, and update `.specs/README.md`’s change-spec index.
- [ ] Add documentation/link checks covering canonical headings, release snippets, and the published command examples; retain the explicit sibling boundary in installer-related prose.

## Definition of done

- [ ] Canonical pages state the implemented provenance, lockfile, permissions, advisory, and hygiene invariants without claiming unbuilt controls.
- [ ] Public binary and container instructions show a concrete verifying path; links and command snippets resolve from their authored locations.
- [ ] `.gitignore` uses the required patterns (`*.pem`, `*.p8`, `*.key`, `keys/`, `data/`, `lmdb/`, `*.db`, `*.sqlite`, `*.sqlite3`) and tracked-file checks show no accidental masking.
- [ ] Change-spec merge/index updates occur only when all in-scope packages are merged; the sibling fail-closed behavior remains separately owned.
- [ ] Documentation and static link checks cover positive references and a broken-link negative fixture where supported.
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: a reviewer can follow the documented binary/container verification instructions and trace each canonical rule to an implemented workflow or installer control.

## Sibling boundaries

- Do not update canonical prose to claim the installer fails closed when no checksum utility exists; that promise and its implementation stay with the unstacked fail-closed sibling.
