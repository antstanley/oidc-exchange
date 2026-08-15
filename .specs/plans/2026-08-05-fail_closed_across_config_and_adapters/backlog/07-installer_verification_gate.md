# Task 07 — Installer verification gate

**Plan:** [plan.md](../plan.md)  
**Status:** Backlog  
**Implements:** [source spec](../../../changes/2026-08-05-fail_closed_across_config_and_adapters.md) → Distribution Install script/Decision and Implementation note 7; [distribution canonical page](../../../bindings/specs/05-distribution.md) → Install script and Checksum-verified install decision  
**Depends on:** —  
**Produces:** `install.sh` reports missing checksum support on stderr and exits non-zero before chmod/move; no path installs a binary whose checksum could not be verified.  
**Pointers:** `install.sh`; existing shell-test convention or a narrowly added hermetic test harness; release asset download behavior (read-only).

## Steps

- [ ] Replace the missing-`sha256sum`/`shasum` warning branch with a stderr error and non-zero
  exit before the installation section; keep successful verifier behavior unchanged.
- [ ] Add a hermetic shell test that masks both checksum tools from `PATH`, supplies/mocks all
  earlier commands and downloads deterministically, asserts non-zero exit, and asserts no binary
  is chmodded or moved into the install path.
- [ ] Add companion tests for each supported checksum utility and a bad checksum, proving success
  only follows successful verification and a verifier failure cannot reach installation.
- [ ] Keep shell error handling (`set -euo pipefail`) effective and avoid leaking temporary files
  or writing outside a test-controlled install directory.

## Definition of done

- [ ] A host without both supported checksum utilities receives a clear stderr diagnostic and no
  installed binary.
- [ ] A failed checksum utility also cannot reach chmod/move; either supported verifier can still
  install a verified fixture.
- [ ] Tests are hermetic, clean their temporary paths, and do not use network/GitHub Releases.
- [ ] Shell formatting/linting available to the repository and the focused harness results are
  reported.

## Sibling boundaries

- Do not modify `--version` parsing, release URL traversal, checksum signing, attestations, or
  release workflow provenance. Those belong to the release-supply-chain sibling.
