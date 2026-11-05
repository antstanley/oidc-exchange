---
"@oidc-exchange/node": patch
---

Download-host zone name and npm dist-tag for pre-releases

Two release-readiness fixes found while preparing the first release with the new pipeline.

**Download-host zone**
The account has no `iamstan.dev` hosted zone; `dl.oidc-exchange.iamstan.dev` belongs in the existing `oidc-exchange.iamstan.dev` zone (`Z03720342C1JO5SG517CQ`). The `cdk.json` default and the `06-download-host.md` runbook pointed at the wrong zone name. Updated both and the stack test fixture. (The stack is already deployed with the correct context.)

**npm dist-tag for pre-releases**
`publish-npm` called `npm stage publish` without `--tag`, so an approved pre-release such as `v0.5.0-rc.1` would become `latest` for every user. A new step derives the tag from the release version (`1.2.3` → `latest`, `1.2.3-rc.1` → `rc`, `1.2.3-beta.4` → `beta`) and all three stage calls pass it. Ported from blogwright's `release.yml`; `05-distribution.md` notes it.

**Validation**
`pnpm test:supply-chain` (54 pass), stack tests and `cdk synth`. The derivation was checked against `1.2.3`, `1.2.3-rc.1`, `1.2.3-beta.4` and `1.2.3-alpha`; the workflow change itself only runs on a real tag.
