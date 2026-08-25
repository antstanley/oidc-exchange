# Task 11 — Reference conformance gate

**Plan:** [plan.md](../plan.md) · **Certificate:** omitted by requester

**Implements:** [change spec §Proposed changes — Reference deployments / Conformance gate](../../../changes/2026-08-05-baseline_reference_deployments.md#proposed-changes), [change spec §Implementation notes — C gate](../../../changes/2026-08-05-baseline_reference_deployments.md#implementation-notes), [change spec §Regression tests](../../../changes/2026-08-05-baseline_reference_deployments.md#regression-tests), [bindings distribution §Release pipeline](../../../bindings/specs/05-distribution.md#release-pipeline-githubworkflows)
**Depends on:** 02, 03, 04, 05, 06, 07, 08, 09
**Produces:** a blocking `reference-baseline` CI job that discovers shipped templates, scans B1–B4, checks B5, rejects mutable versions, runs CDK synthesis, and relies on workspace tests for cross-layer behavior.
**Pointers:** `.github/workflows/ci.yml`; `docs/security/reference-baseline.md`; `examples/*/infra/`; `config/`; `examples/`; `docs/`; `crates/adapters/src/valkey/mod.rs`; `crates/adapters/src/sqlite/mod.rs`; `crates/adapters/src/postgres/mod.rs`

## Pre-merge verification (2026-08-25)

The consumed checker was exercised against post-merge `main` before this plan
landed: every shipped TOML under `config/` and `examples/` resolves —
placeholder-free files through the positional env-free `config check <path>`
form, placeholder-bearing files through `config check --file` with their
`${VAR}`s supplied. The sweep surfaced (and the merge fixed) one closed-domain
drift: `telemetry.exporter = "xray"` in `examples/aws-web/config/
oidc-exchange.toml` was documented and handled by the telemetry layer but
missing from the typed `TelemetryExporter` domain. The gate this task builds
would have caught exactly that class of drift; the premise is validated.

## Steps

- [ ] Select and version-pin a scanner only after mapping each rule to one B1–B4 statement in Task 08’s baseline; implement an in-tree rationale-required exception mechanism.
- [ ] Add `reference-baseline` CI setup that discovers Terraform roots/templates from the filesystem, scans Terraform/CDK/Kubernetes/compose inputs, runs credential-free CDK synth, and rejects synthesized secret literals.
- [ ] Consume the now-merged `oidc-exchange config check` (positional env-free form for shipped files; `--dir`/`--file` for env-aware layerings) over every shipped TOML under `config/`, `examples/`, and `docs/`; fail mutable image references and Terraform roots without committed locks.
- [ ] Ensure the Rust workspace `test` job contains the Valkey TLS, SQLite/LMDB modes, and Postgres convergence assertions rather than trying to encode them as static scanner rules.
- [ ] Land reporting mode only long enough to triage known findings, then make the job blocking after every scoped template conforms; document any approved exception in-tree.

## Definition of done

- [ ] CI discovers rather than hardcodes template coverage, so a newly added deployable template is automatically assessed.
- [ ] Every scanner rule traces to B1–B4, every exception has an in-tree rationale, and unexplained exceptions fail.
- [ ] Config checks, immutable-reference checks, Terraform lock checks, CDK synth, and secret-literal detection run in the job and reject negative fixtures.
- [ ] Cross-layer workspace tests prove Valkey TLS selection, restrictive local-state modes, and Postgres example-schema convergence.
- [ ] The job is blocking only after remediation is green; it does not silently waive failures or absorb sibling implementation work.
- [ ] Meets the repo definition of done (CI YAML/action pinning, Rust/TypeScript/Python checks as touched, relevant scanner/config checks, negative-space tests, and named-constant limits — see plan.md baseline).
- [ ] Reviewable: open a PR with a mutable image, missing lockfile, plaintext/secret template regression, and invalid TOML fixture and observe the blocking gate identify each violation.
