# Done Certificate — Task 01: sweep stale CloudTrail and atproto-as-shipped references

**Task:** [01-sweep_stale_references.md](01-sweep_stale_references.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-06-29

> This certificate is a verification protocol for Task 01. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 01) ≡ every obligation O1…O5 below holds, each backed by the evidence the obligation
names (a grep result, a build/lint result, or a file read) — not by assertion.

## Premises

- **P1 — Goal.** The task produces docs, example configs, the `aws-web` CDK example, and
  `README.md` with no stale `cloudtrail` audit-adapter or atproto-as-shipped references; every
  example `[audit]` block selects a real `noop`/`stdout`/`sqs` adapter and atproto reads as planned.
- **P2 — Obligations.** The task is done iff O1…O5 all hold. One Oi per definition-of-done item, in
  DoD order; O5 is the `Reviewable:` item.
- **P3 — Invariants.** Must not break the canonical spec bodies (06-configuration's `[audit]` /
  `[providers.<name>]` enumerations, 00-overview) — docs are aligned *to* them, never contradicting
  them — nor the Astro docs site build (the `apps/website/src/content/docs` symlink → `docs/`), nor
  the TOML parseability of the example configs, nor the CDK example's TypeScript validity.

## Obligations

- **O1 — No stale cloudtrail/atproto adapter or provider tokens remain.**
  - *Claim:* the four stale selector tokens are gone from all non-spec material.
  - *Evidence to collect:* run
    `rg -n 'adapter = "cloudtrail"|\[audit\.cloudtrail\]|adapter = "atproto"|\[providers\.atproto\]' docs/ examples/ config/ apps/ README.md`
    from the workspace root — expect **no matches** (exit code 1, empty output).
  - *Status:* SATISFIED

- **O2 — Every example/doc audit block selects a real adapter; no CloudTrail audit construct remains in `aws-web`.**
  - *Claim:* each `[audit]` block in docs/examples uses `noop`, `stdout`, or `sqs` (with a matching
    `[audit.sqs]` wherever `sqs` is chosen), and `examples/aws-web` carries no `channel_arn` / CDK
    CloudTrail audit construct.
  - *Evidence to collect:* read each `[audit]` block at `docs/deployment/aws-lambda.md` (~L86),
    `docs/guides/configuration.md` (~L102), `docs/architecture/adapters.md` (audit sections),
    `examples/aws-web/config/oidc-exchange.toml` (~L26) and confirm the adapter value is one of
    `noop`/`stdout`/`sqs`; run `rg -n 'cloudtrail|channel_arn|CfnEventDataStore|CfnChannel|cloudtrail-data' examples/aws-web/`
    — expect **no matches**; confirm `examples/aws-web/config/oidc-exchange.toml` with `adapter = "sqs"`
    has a corresponding `[audit.sqs]` block.
  - *Checks:* resolve the audit env wiring in `examples/aws-web/infra/lib/stack.ts` — confirm the
    queue URL the config references (e.g. `AUDIT_QUEUE_URL` / `OIDC_EXCHANGE__AUDIT__SQS__QUEUE_URL`)
    is actually produced by the SQS queue construct that replaced the CloudTrail constructs, not a
    dangling reference to a removed resource.
  - *Status:* SATISFIED

- **O3 — Every remaining atproto mention reads as planned.**
  - *Claim:* no surviving `atproto` occurrence describes it as a shipped or selectable provider/adapter.
  - *Evidence to collect:* run `rg -ni 'atproto' docs/ apps/ README.md` and read each hit in context;
    confirm every one is qualified as planned / not-yet-implemented (e.g. "planned", "not yet
    implemented", a pointer to the atproto change spec) and none presents `adapter = "atproto"` as a
    working choice or lists atproto among shipped adapters without qualification.
  - *Status:* SATISFIED

- **O4 — Meets the repo definition of done for a docs/config change.**
  - *Claim:* TOML stays parseable, the touched TypeScript passes format/lint, and the docs site builds.
  - *Evidence to collect:* per `.specs/development-guidelines.md` §Definition of done (TypeScript
    row), run `pnpm fmt:check` and `pnpm lint` — expect clean for the edited
    `examples/aws-web/infra/lib/stack.ts`; run `pnpm --filter ./apps/website build` — expect a
    successful build; spot-confirm the edited TOML files parse (the website build / any TOML lint, or
    `python -c "import tomllib,sys;[tomllib.load(open(f,'rb')) for f in sys.argv[1:]]" examples/aws-web/config/oidc-exchange.toml config/default.toml`).
    (No Rust is touched — the Rust DoD row does not apply.)
  - *Status:* SATISFIED

- **O5 — Reviewable: grep is clean, the site builds, and the aws-web audit sink is internally consistent (Reviewable).**
  - *Claim:* a reviewer can confirm no stale tokens remain, the docs site builds, and the rewritten
    `aws-web` config + CDK stack describe one coherent audit sink.
  - *Evidence to collect:* run the O1 `rg` command (expect no hits) and
    `pnpm --filter ./apps/website build` (expect clean); read
    `examples/aws-web/config/oidc-exchange.toml` and `examples/aws-web/infra/lib/stack.ts` together
    and confirm the config's chosen adapter and the infra it provisions agree (SQS queue ↔
    `[audit.sqs]` + `sqs:SendMessage` grant + wired queue URL).
  - *Status:* SATISFIED

## Regression check

This task edits documentation, example configs, and one CDK example; it touches no shipped runtime
code. The invariant surface is the canonical spec and the build, not call paths:

- The Astro docs site (`apps/website`, consuming `docs/` via the content symlink) still builds after
  the edits → `corepack pnpm --filter ./apps/website build` completed, 22 pages, "Complete!" : PRESERVED
- The edited docs do not contradict the canonical `06-configuration.md` `[audit]` / `[providers]`
  enumerations or `00-overview.md` → audit blocks select only noop/stdout/sqs, providers only
  oidc/apple, every atproto mention reads planned : PRESERVED

## Residue

Notes for the validator, not obligations:

- The change spec leaves a CloudTrail-migration note for former `cloudtrail` users as an undecided
  open question; this task does **not** add one (see plan.md Open questions). Its absence is not a
  defect.
- `config/default.toml` has no stale audit comment to fix (Implementation note 3 already satisfied);
  do not flag its absence from the diff.
- Legitimate, non-audit CloudTrail mentions (if any survive purely as a downstream-ingestion
  concept) are acceptable only if they no longer present a `cloudtrail` audit *adapter*; O1's token
  set is the contract.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: All five obligations SATISFIED — the four stale selector tokens are gone (O1 grep empty), every example/doc audit block selects noop/stdout/sqs with a matching [audit.sqs] and no CloudTrail construct survives in aws-web (O2), all eight remaining atproto mentions read as planned (O3), TOML parses and `pnpm --filter ./apps/website build` is clean with the TS edit type-correct (O4), and the aws-web config + CDK stack describe one coherent SQS audit sink (O5); regression PRESERVED.
