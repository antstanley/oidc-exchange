# Task 02 — Fargate transport

**Plan:** [plan.md](../plan.md) · **Certificate:** omitted by requester

**Implements:** [change spec §Implementation notes — A.1 Fargate listener chain](../../../changes/2026-08-05-baseline_reference_deployments.md#implementation-notes), [change spec §Implementation notes — A.3 Valkey](../../../changes/2026-08-05-baseline_reference_deployments.md#implementation-notes), [change spec §Compatibility](../../../changes/2026-08-05-baseline_reference_deployments.md#compatibility)
**Depends on:** 01
**Produces:** an ECS/Fargate Terraform template whose listener, output, Valkey, secret, lockfile, and documentation agree on TLS-first transport.
**Pointers:** `examples/ecs-fargate/infra/main.tf:268-282,541-565,613-679`; `examples/ecs-fargate/infra/variables.tf:35-39`; `examples/ecs-fargate/infra/outputs.tf:1-4`; `examples/ecs-fargate/infra/terraform.tfvars.example`; `examples/ecs-fargate/README.md:73-75`; `docs/deployment/ecs-fargate.md:116-118`

## Steps

- [ ] Add the testing-only `allow_insecure_http` variable, invert listener conditions so certificate-backed TLS is normal, HTTP redirects when TLS exists, and forwarding HTTP requires explicit opt-in when no certificate exists.
- [ ] Repair dependent resource references and `alb_url` scheme rendering so every valid Terraform plan has exactly one port-80 listener and documentation shows the provisioned scheme.
- [ ] Enable ElastiCache transit and at-rest encryption, create/store its auth token in Secrets Manager, and place an authenticated `rediss://` URL in the ECS secrets block.
- [ ] Add the committed Terraform lockfile, replace mutable image references in Fargate files, and update operator documentation including the loopback-only local `redis://` rationale.
- [ ] Add Terraform plan/assertion coverage for certificate and insecure-opt-in permutations plus encryption, secret, and output behavior.

## Definition of done

- [ ] Plans with a certificate always redirect port 80 and publish an HTTPS URL; plans without a certificate forward HTTP only with `allow_insecure_http = true`.
- [ ] The ECS service no longer depends on an absent conditional listener, and all valid listener paths plan successfully.
- [ ] The template declares encrypted/authenticated Valkey only after Task 01 proves the client honors `rediss://`.
- [ ] Documentation describes the compatibility change and contains no contradictory plaintext quick-start URL.
- [ ] Meets the repo definition of done (applicable Terraform/template checks plus Rust/TypeScript checks touched by this task; negative-space tests; named-constant limits where introduced — see plan.md baseline).
- [ ] Reviewable: run the four Terraform plan permutations and inspect listener actions, `alb_url`, and the ECS secret wiring.
