# Task 05 — Multi-graph advisory gate

**Plan:** [plan.md](../plan.md)

**Implements:** [source spec](../../../changes/2026-08-05-harden_release_supply_chain.md) §Proposed changes → Supply-chain gates → Advisories/All three dependency graphs; §Implementation notes C.9–C.11; [development guidelines](../../../development-guidelines.md) §Repository hygiene
**Depends on:** 01
**Produces:** CI and release pre-publish gates evaluate Cargo, pnpm, and Python dependency findings against recorded, dated policy rather than leaving advisories silent.
**Pointers:** `deny.toml` (new); `.github/workflows/ci.yml:12-148`; `.github/workflows/release.yml:16-618`; `Cargo.lock`; `bindings/python/Cargo.toml:12`; `bindings/python/pyproject.toml`; `bindings/lambda/pnpm-lock.yaml`

## Steps

- [ ] Create `deny.toml` with the named Cargo advisory entries, reachability rationales, and expiry dates; distinguish advisory failures from unmaintained/yanked warnings.
- [ ] Add reporting-mode Cargo, pnpm, and Python audit steps to a dedicated CI advisories job, using committed lockfiles/environments and conventional per-ecosystem ignore locations rather than treating `deny.toml` as universal.
- [ ] Add equivalent release gates before publishing work and transition unrecorded advisories to blocking behavior after the baseline findings are explicitly recorded.
- [ ] Upgrade `pyo3` past the documented advisory line, confirm ABI/maturin compatibility, regenerate lockfiles as required, and remove now-obsolete Cargo ignore entries.
- [ ] Add deterministic fixtures or command-wrapper tests for missing policy entries, unexpired documented entries, expired entries, warning-only unmaintained/yanked findings, and each graph’s reporting-to-blocking behavior.

## Definition of done

- [ ] Cargo, pnpm, and Python dependency graphs are each inspected in CI and before publishing, with reproducible inputs.
- [ ] Every carried advisory has a falsifiable rationale and expiry; unknown and expired advisories fail while unmaintained/yanked findings warn.
- [ ] `pyo3` no longer remains on the specified vulnerable line, and the supported abi3/maturin build is proven after the update.
- [ ] Positive and negative audit-policy tests demonstrate the stated pass, warn, and fail outcomes without requiring live publication.
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: a reviewer can inspect the dated policy and run fixtures that show unrecorded findings fail while recorded bounded findings follow the documented outcome.

## Sibling boundaries

- Do not fold generic config/adapters fail-closed work into the audit job; this task owns dependency graph policy only.
