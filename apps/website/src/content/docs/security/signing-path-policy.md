---
title: Signing Path Policy
---

# Signing-path dependency policy

**Effective:** 2026-08-23  
**Owner:** Ant Stanley

Roots come from code, not package-name inference. `LocalKeyManager` in `crates/adapters/src/local_keys/mod.rs` signs and verifies with `ed25519-dalek`. `KmsKeyManager` in `crates/adapters/src/kms/mod.rs` calls AWS KMS Sign and locally parses/verifies RSA and P-256/P-384/P-521 signatures. Therefore the resolved root is `oidc-exchange-adapters` at the workspace version in both all-target/all-feature workspace metadata and the Linux release target metadata. `config/signing-path-policy.json` pins that version in its roots and exception paths, and `scripts/post-changeset-version.mjs` moves it with every release. Test-only OIDC private-key helpers are not deployment roots.

`cargo metadata --locked --format-version 1 --all-features` is traversed for all targets and again with `--filter-platform x86_64-unknown-linux-gnu`. Normal and build edges are included; dev edges are excluded. Every reachable protected cryptographic package is checked for SemVer prerelease identifiers. Diagnostics include an exact root-to-package path. Missing roots, nodes, packages, dependency kinds, malformed JSON, metadata command failure, and mode changes fail closed. Exact path matching means a new feature, target, version, or transitive path cannot inherit an old exception.

The current prerelease inventory in each mode is one direct dependency, `rsa 0.10.0-rc.20`, which the KMS key manager uses only to parse public keys and verify signatures. `ed25519-dalek`, `p256`, `p384`, `p521`, `curve25519-dalek`, and `ecdsa` are on stable releases. Two dated exceptions (one per mode) cover `rsa`, record the exact root path, and expire 2027-01-15, with review due 2026-12-01. This is narrow baseline triage, not an upstream-remediation claim or a blanket prerelease allowance.

Run `node scripts/run-signing-path-policy.mjs` after any Cargo feature, target, root, or lockfile change. Before extending an exception, inspect the source operation, regenerate both metadata modes, confirm every path, assign an owner and near-term expiry, and prefer a separately reviewed stable dependency migration.
