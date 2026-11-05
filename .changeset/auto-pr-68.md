---
"@oidc-exchange/node": patch
---

Default download-host zone to oidc-exchange.iamstan.dev

The account has no `iamstan.dev` hosted zone; `dl.oidc-exchange.iamstan.dev` belongs in the existing `oidc-exchange.iamstan.dev` zone (`Z03720342C1JO5SG517CQ`). The `cdk.json` default and the `06-download-host.md` runbook both pointed at the wrong zone name, which would have put the ACM validation records outside the real zone.

- `infra/downloads/cdk.json`: default `zoneName` → `oidc-exchange.iamstan.dev`
- `06-download-host.md`: runbook command and a note on the zone
- stack test fixture updated to match

No resource changes: a `cdk diff` against the account with the correct context is purely additive. `pnpm test`, `cdk synth` and format check pass.
