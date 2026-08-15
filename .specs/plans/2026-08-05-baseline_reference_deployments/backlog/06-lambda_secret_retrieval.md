# Task 06 — Lambda secret retrieval

**Plan:** [plan.md](../plan.md) · **Certificate:** omitted by requester

**Implements:** [change spec §Implementation notes — A.2 CDK client secret](../../../changes/2026-08-05-baseline_reference_deployments.md#implementation-notes), [change spec §Regression tests](../../../changes/2026-08-05-baseline_reference_deployments.md#regression-tests), [change spec §Compatibility](../../../changes/2026-08-05-baseline_reference_deployments.md#compatibility)
**Depends on:** —
**Produces:** an AWS Lambda reference deployment that grants read access to a supplied Secrets Manager ARN and fetches the Google secret before service configuration resolves its placeholder.
**Pointers:** `examples/aws-web/infra/bin/app.ts:8-12`; `examples/aws-web/infra/lib/stack.ts:13,52-72,89-93,159-166`; `examples/aws-web/infra/cdk.json`; `examples/aws-web/README.md`; `crates/server/src/bootstrap.rs:164-197`

## Steps

- [ ] Replace plaintext CDK context and Lambda environment injection with a required `googleClientSecretArn`, imported Secrets Manager secret, and least-privilege read grant to the auth function.
- [ ] Package an `AWS_LAMBDA_EXEC_WRAPPER` that resolves the referenced secret, exports `GOOGLE_CLIENT_SECRET`, and execs the existing bootstrap before config placeholder loading.
- [ ] Remove committed secret placeholders/value paths from CDK configuration and update the AWS example instructions to supply an ARN and rotate previously exposed secrets.
- [ ] Add synth-level tests/assertions that secret values cannot appear in CloudFormation output and runtime wiring has only the reference plus permission/wrapper behavior.

## Definition of done

- [ ] CDK configuration fails fast when the secret ARN is absent and no secret value is accepted as context.
- [ ] Synthesized templates contain no Google secret literal, while the auth function has only the required ARN/reference and read permission.
- [ ] The wrapper makes `${GOOGLE_CLIENT_SECRET}` available before existing service placeholder resolution, with failures surfaced rather than silently ignored.
- [ ] Negative fixtures prove both missing ARN and injected literal secret are rejected by tests/checks.
- [ ] Meets the repo definition of done (TypeScript format/lint/typecheck/tests and applicable shell/CDK checks; negative-space tests; named-constant limits where introduced — see plan.md baseline).
- [ ] Reviewable: run CDK synth, search the output for the supplied secret value, and inspect the wrapper/role relationship.
