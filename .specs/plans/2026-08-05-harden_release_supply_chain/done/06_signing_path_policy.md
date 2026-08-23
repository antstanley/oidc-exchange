# Task 06 — Signing-path policy

**Plan:** [plan.md](../plan.md)

**Implements:** [source spec](../../../changes/merged/2026-08-05-harden_release_supply_chain.md) §Implementation notes D.12; §Proposed changes → Assumptions / Decisions → Open questions; [distribution canonical page](../../../bindings/specs/05-distribution.md) §Supply-chain gates
**Depends on:** 05
**Produces:** a resolved-graph policy check prevents signing and verification paths from silently selecting pre-release cryptographic dependencies.
**Pointers:** `Cargo.lock`; `config/signing-path-policy.json` (2 modes, 14 exact path exceptions); `crates/adapters/Cargo.toml:14-22,40`; `crates/adapters/src/local_keys/mod.rs:1-105`; `crates/adapters/src/kms/mod.rs:1-379,637-900`; `crates/adapters/src/oidc/mod.rs:268-333`

## Steps

- [x] Define the deployment-mode signing/verification roots from the current adapter source and resolve their transitive Cargo packages from `Cargo.lock`, not manifest text alone.
- [x] Implement a deterministic policy checker with an explicit allowlist/root definition and a clear failure diagnostic for a selected pre-release cryptographic dependency.
- [x] Run the check in the relevant CI and release advisory/policy gate after task 05 establishes the dependency-policy execution path.
- [x] Add fixtures covering stable acceptable packages, a direct pre-release, a transitive pre-release, duplicate major lines where only one is reachable from a protected root, and a non-signing pre-release that must not create a false positive.

## Definition of done

- [x] The check reasons over the resolved graph and reports the protected root-to-package path for each failure.
- [x] A pre-release package reachable from an actual signing/verification root fails; equivalent unreachable or unrelated packages do not.
- [x] Deployment-mode roots, exemptions, and the source evidence for them are explicit and reviewable.
- [x] Positive and negative fixtures cover duplicate version lines and transitive resolution.
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: a reviewer can modify a lockfile fixture to place a release candidate on a protected path and observe a path-specific failure.

## Sibling boundaries

- Do not use this policy task to upgrade unrelated crypto crates or resolve the sibling’s installer fail-closed issue; it owns detection on the defined signing/verification path.

## Review-round-1 remediation evidence

- Traversal skips an edge only when all applicable dependency kinds are dev-only; mixed normal/dev, null-as-normal, build, and target-qualified fixtures retain runtime edges. Both real modes report the same seven protected prereleases, including runtime `rsa 0.10.0-rc.18`; all 14 exact path exceptions are consumed with no failures or stale entries. RSA private-key construction is test-only, but public-key verification ships.
