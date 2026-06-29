# Task 01 — Sweep stale CloudTrail and atproto-as-shipped references

**Plan:** [plan.md](../plan.md) · **Certificate:** [01-sweep_stale_references-certificate.md](01-sweep_stale_references-certificate.md)

**Implements:** [changes/2026-06-24-cleanup_stale_references.md](../../../changes/2026-06-24-cleanup_stale_references.md) §Implementation notes 1–5; aligns to [service/specs/06-configuration.md](../../../service/specs/06-configuration.md) §`[audit]` and §`[providers.<name>]` and [service/specs/00-overview.md](../../../service/specs/00-overview.md) (atproto not implemented)
**Depends on:** —
**Produces:** docs, example configs, the `aws-web` CDK example, and `README.md` carry no stale `cloudtrail` audit-adapter or atproto-as-shipped references; every example `[audit]` block selects a real `noop`/`stdout`/`sqs` adapter, and atproto reads as planned everywhere it appears
**Pointers:** `docs/deployment/aws-lambda.md:86`, `docs/guides/configuration.md:104`, `docs/guides/providers.md:116`, `docs/architecture/adapters.md:159`, `docs/architecture/overview.md:15`, `examples/aws-web/config/oidc-exchange.toml:26`, `examples/aws-web/infra/lib/stack.ts:4`, `README.md:65`

## Steps

- [ ] Re-run the discovery greps to get the live hit list: `rg -n 'cloudtrail' docs/ examples/ config/ apps/ README.md`, `rg -ni 'atproto' docs/ apps/ README.md`, and `rg -n 'adapter = "(file|webhook)"' -g '*.toml'` (expected: no file/webhook hits).
- [ ] `docs/deployment/aws-lambda.md` — switch the `[audit]` block from `adapter = "cloudtrail"` + `[audit.cloudtrail]`/`channel_arn` to `adapter = "sqs"` + `[audit.sqs]`/`queue_url`; in the IAM section replace `cloudtrail-data:PutAuditEvents` with `sqs:SendMessage` on the audit queue.
- [ ] `docs/guides/configuration.md` — fix the audit-adapter comment to `# "noop", "stdout", or "sqs"`; replace the `[audit.cloudtrail]`/`channel_arn` example block with an `[audit.sqs]`/`queue_url` block; remove the runnable `[providers.atproto]` example block (it selects a non-existent adapter) and, if useful, leave a one-line "atproto is planned — see the atproto change spec" pointer instead.
- [ ] `docs/guides/providers.md` — change the provider-adapter comment from `# "oidc", "apple", or "atproto"` to list only the implemented adapters (`"oidc"` or `"apple"`), noting atproto is planned.
- [ ] `docs/architecture/adapters.md` — replace the "CloudTrail Lake" audit section (heading + `adapter = "cloudtrail"` block) with a "Stdout/Stderr" section documenting `adapter = "stdout"`, so the page enumerates the real noop/stdout/sqs trio.
- [ ] `docs/architecture/overview.md` — in the crate-tree and crate-table adapter lists replace `CloudTrail` with the real audit sinks (stdout/stderr, SQS); in the `AuditLog` ports-table row change `Noop, CloudTrail Lake, SQS` to `Noop, Stdout/Stderr, SQS`; qualify atproto as planned in the `providers/` tree line, the crate-table providers row, and the `IdentityProvider` row.
- [ ] `examples/aws-web/config/oidc-exchange.toml` — switch the `[audit]` block to `adapter = "sqs"` + `[audit.sqs]`/`queue_url` (driven by an env placeholder such as `${AUDIT_QUEUE_URL}`).
- [ ] `examples/aws-web/infra/lib/stack.ts` — replace the CloudTrail `CfnEventDataStore`/`CfnChannel` constructs and the `cloudtrail-data:PutAuditEvents` IAM statement with an SQS queue, grant the function `sqs:SendMessage` on it, and wire its URL into the function config (e.g. `OIDC_EXCHANGE__AUDIT__SQS__QUEUE_URL` / the `AUDIT_QUEUE_URL` the config references); drop the now-unused `aws-cloudtrail` import.
- [ ] `README.md` — reword the audit-trail bullet and Features line away from "CloudTrail Lake" to the implemented sinks (stdout/stderr, SQS); in the architecture tree replace `CloudTrail` in the adapters comment; in the ports table change the `AuditLog` adapters to `Noop, Stdout/Stderr, SQS`; ensure every atproto mention (architecture tree, features) reads as planned.
- [ ] Rebuild the docs site to confirm nothing broke: `pnpm --filter ./apps/website build` (the `apps/website/src/content/docs` symlink points at `docs/`, so edits flow through; a clean build is the check).

## Definition of done

- [ ] `rg -n 'adapter = "cloudtrail"|\[audit\.cloudtrail\]|adapter = "atproto"|\[providers\.atproto\]' docs/ examples/ config/ apps/ README.md` returns no matches.
- [ ] Every example/doc `[audit]` block selects `noop`, `stdout`, or `sqs` (with a matching `[audit.sqs]` where `sqs` is chosen), and no `channel_arn`/CloudTrail audit construct remains in `examples/aws-web`.
- [ ] Every remaining `atproto` mention is qualified as planned / not-yet-implemented (none described as a shipped or selectable adapter).
- [ ] Meets the repo definition of done for a docs/config change (see plan.md baseline): TOML stays parseable, the touched TypeScript passes `pnpm fmt:check`/`pnpm lint`, and `pnpm --filter ./apps/website build` succeeds.
- [ ] Reviewable: run the grep above (no hits) and `pnpm --filter ./apps/website build` (clean), and read the edited `aws-web` config + CDK stack to confirm the audit sink is internally consistent.

## Open questions

- None.
