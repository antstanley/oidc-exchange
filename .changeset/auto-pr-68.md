---
"@oidc-exchange/node": patch
---

Download-host zone name and npm dist-tag for pre-releases

Two release-readiness fixes found while preparing the first release with the new pipeline.

**Download-host zone**
The account has no `iamstan.dev` hosted zone; `dl.oidc-exchange.iamstan.dev` belongs in the existing `oidc-exchange.iamstan.dev` zone (`Z03720342C1JO5SG517CQ`). The `cdk.json` default and the `06-download-host.md` runbook pointed at the wrong zone name. Updated both and the stack test fixture. The stack is already deployed with the correct context; the deployed A/AAAA and ACM validation records match.

**npm dist-tag for pre-releases**
`npm stage publish` refuses a pre-release without an explicit `--tag`, so a `v0.5.0-rc.1` tag would have failed at the first stage call. A new `publish-npm` step derives the tag from the release version and all three stage calls pass it:
- `1.2.3-rc.1` → `rc`, `1.2.3-beta.4` → `beta`, `0.5.0-next.0` → `next`
- numeric or range-like first identifiers (`1.2.3-0`, `1.2.3-1.rc`, `1.2.3-x.1`) → `next`, because npm rejects a tag that is a valid semver range
- stable releases pass **no** `--tag`, so npm's own guard still stops a lower stable version from silently moving `latest` backwards

Ported from blogwright's `release.yml`, then hardened after review. `05-distribution.md` describes it.

…
