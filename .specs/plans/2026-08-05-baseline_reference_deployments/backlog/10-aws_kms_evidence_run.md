# Task 10 — AWS KMS evidence run

**Plan:** [plan.md](../plan.md) · **Certificate:** omitted by requester

**Implements:** [change spec §Implementation notes — D manual KMS run](../../../changes/2026-08-05-baseline_reference_deployments.md#implementation-notes), [change spec §Assumptions and open questions — AWS reference deployments](../../../changes/2026-08-05-baseline_reference_deployments.md#assumptions-and-open-questions)
**Depends on:** —
**Produces:** a dated, reproducible record of first-request `/token` and `/keys` behavior for both AWS reference deployments using their shipped KMS algorithm strings.
**Pointers:** `examples/aws-web/config/oidc-exchange.toml:17`; `examples/ecs-fargate/config/fargate.toml:11`; `.specs/changes/2026-08-05-baseline_reference_deployments.md:566-574`; sibling `2026-08-05-fail_closed_across_config_and_adapters.md` (external dependency)

## Steps

- [ ] Deploy `examples/aws-web` once with its shipped KMS algorithm value and capture setup/version identifiers, first `/token` request, and first `/keys` request outputs with secrets redacted.
- [ ] Deploy `examples/ecs-fargate` once with its shipped KMS algorithm value and capture the equivalent first-request evidence.
- [ ] Record pass/fail results, environment prerequisites, command provenance, and any reproducible error signatures in an in-tree evidence location agreed by the maintainer.
- [ ] Report algorithm-related failures to the sibling fail-closed change without patching KMS algorithm strings in this PR.

## Definition of done

- [ ] Evidence identifies both deployed template revisions, the exact shipped algorithm strings, endpoint requests, outcomes, and redacted logs/traces.
- [ ] The record distinguishes deployment/template failures from the sibling-owned KMS algorithm defect.
- [ ] No secret, credential, token, or private endpoint data is committed in the evidence.
- [ ] If execution is blocked by AWS access/cost controls, the record names the blocker and required maintainer action instead of inferring success.
- [ ] Meets the repo definition of done (applicable redaction, documentation, and reproducibility review — see plan.md baseline).
- [ ] Reviewable: another maintainer can reproduce the two deployments and compare first-request outcomes against the recorded evidence.
