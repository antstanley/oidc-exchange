# Dependency advisory policy

**Effective:** 2026-08-23  
**Owner:** Ant Stanley  
**Review cadence:** on every exception review date, dependency change, and tagged release

The machine-readable source is `config/advisory-policy.json`. Cargo (`Cargo.lock`), pnpm (`pnpm-lock.yaml`, including every workspace importer), and Python (`bindings/python/uv.lock`) are resolved, committed graphs. Direct and transitive packages receive the same policy. High and critical vulnerabilities fail unless an exact advisory/package/version exception is active. Unmaintained and yanked packages warn. Scanner, network, database-update, malformed-output, and incomplete-output failures fail closed and are never reported as a clean graph.

CI reports allowed exceptions and warnings while still blocking unknown or expired high-severity findings. Tagged release runs the identical evaluation before `validate`, so every build and publish path depends on a successful policy job. Scanner versions are `cargo-deny 0.19.0`, `pnpm 11.9.0`, and `pip-audit 2.9.0`; pip-audit and its dependency closure are provisioned from committed fully hashed binary requirements. Scans consume frozen locks and never run dependency update commands.

Exceptions cannot contain ranges or wildcards. Each records advisory ID, exact package/version and `=version` range, rationale, owner, expiry, and review date. The current inventory is seven Cargo exceptions: Marvin for rsa 0.9.10 and 0.10.0-rc.18, three rustls-webpki 0.101.7 advisories, and h2 0.3.27 plus 0.4.15. The obsolete pyo3 advisory exceptions were removed after upgrading the Python binding to pyo3 0.29.2. Eleven pnpm exceptions cover exact transitive js-yaml, postcss, nanoid, and svgo findings in build/static-asset tooling. There are no Python exceptions. This inventory records current triage; it does not claim upstream remediation.

To review, provision `pip-audit==2.9.0` from `config/pip-audit-requirements.txt` with `uv pip install --require-hashes --only-binary=:all:` in an isolated environment, put that environment on `PATH`, then run `node scripts/run-advisory-scans.mjs`. The frozen Python production export is empty because the abi3 package has no runtime Python dependencies; pip-audit is invoked without dependency resolution so that valid empty graph is still evaluated. Treat a scanner/database/network failure separately from findings. Do not update locks to make the gate green without a separately reviewed dependency change. Before extending an exception, revalidate reachability and record a new bounded date and rationale.
