# Task 03 — Binary and image attestation

**Plan:** [plan.md](../plan.md)

**Implements:** [source spec](../../../changes/2026-08-05-harden_release_supply_chain.md) §Proposed changes → Artifacts and Release pipeline; §Implementation notes B.7; §Regression tests; [distribution canonical page](../../../bindings/specs/05-distribution.md) §Artifacts and §Release pipeline
**Depends on:** 02
**Produces:** tagged binary and container build jobs emit SHA-pinned Sigstore provenance tied to their produced artifact and image digests.
**Pointers:** `.github/workflows/release.yml:77-148`; `.github/workflows/release.yml:149-203`; `.github/workflows/release.yml:205-265`

## Steps

- [ ] Add SHA-pinned `actions/attest-build-provenance` steps after each binary checksum and Docker image-digest production, with task-specific subject paths/digests.
- [ ] Grant `id-token: write` and `attestations: write` only to `build-binaries` and `build-docker`, preserving the permissions model from task 02.
- [ ] Preserve binary checksums as corruption checks and ensure the Docker attestation is bound to the pushed digest rather than a mutable tag.
- [ ] Add a non-publishing scratch-tag integration procedure or hermetic workflow test that verifies the expected binary/image provenance and rejects a one-byte-modified artifact and wrong-repository identity.

## Definition of done

- [ ] Each released binary and pushed per-platform image digest has a provenance step with a full-SHA action reference and the required narrowly scoped permissions.
- [ ] The attestation subject identifies the produced binary or immutable image digest, not only a tag or checksum sidecar.
- [ ] Verification succeeds for the expected scratch artifact/image and fails for modified content and an unexpected repository.
- [ ] Existing checksum artifact publication remains intact as an integrity fallback.
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: a reviewer can inspect an attestation bound to a release digest and reproduce pass/fail verification cases.

## Sibling boundaries

- Do not alter the sibling’s checksum-tool availability policy; this task creates provenance for the release artifacts only.
