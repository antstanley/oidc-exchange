# Task 04 — Installer provenance verification

**Plan:** [plan.md](../plan.md)

**Implements:** [source spec](../../../changes/2026-08-05-harden_release_supply_chain.md) §Proposed changes → Install script; §Implementation notes A.1 and B.8; §Regression tests; [distribution canonical page](../../../bindings/specs/05-distribution.md) §Install script
**Depends on:** 02, 03
**Produces:** installer rejects unsafe release pins before URL construction and verifies binary provenance with `gh`, while clearly labeling checksum-only fallback as unauthenticated.
**Pointers:** `install.sh:1-116`; `README.md:11-23`; `docs/getting-started/quick-start.md:13-25`; `.github/workflows/release.yml:578-618`

## Steps

- [ ] Parse `--version` defensively, report a usage error for a missing operand, and validate supplied pins against the specified release-tag pattern before constructing a request URL.
- [ ] Add the `gh attestation verify <binary> --repo antstanley/oidc-exchange` path after download and before installation; make verification failure stop installation.
- [ ] Retain checksum verification as the no-`gh` fallback and print a clear corruption-only/not-authenticated diagnostic without changing the absent-checksum-tool branch owned by the sibling.
- [ ] Add hermetic shell tests that intercept network/install side effects and prove unsafe version values (`../`, URL, leading slash), missing operand, attestation failure, and wrong repository stop before installation; cover successful attestation and explicit fallback behavior.

## Definition of done

- [ ] Invalid supplied pins and a bare `--version` exit non-zero with a useful diagnostic before any fetch or install side effect.
- [ ] With `gh` available, successful provenance verification is required before chmod/move; failed or wrong-repository verification cannot install the fixture.
- [ ] Without `gh`, a successful checksum path emits the explicit unauthenticated fallback message; this task does not change the no-checksum-utility result.
- [ ] Tests are hermetic, clean temporary paths, and do not download a real release or write to a real install directory.
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: a reviewer can run the shell harness and observe rejected traversal input, authenticated install, failed provenance, and loud checksum-only fallback.

## Sibling boundaries

- Do not make `sha256sum`/`shasum` absence fail closed or edit its tests; that exact branch is owned by the unstacked fail-closed sibling’s installer task. Coordinate the shared `install.sh` ordering at integration.
