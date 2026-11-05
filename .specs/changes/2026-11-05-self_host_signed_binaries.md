# Change: Self-host signed release binaries at dl.oidc-exchange.iamstan.dev

**Status:** Proposed · **Date:** 2026-11-05 · **Owner:** Ant Stanley · **Target:** bindings (distribution), `.github/workflows`, `install.sh`, `infra/downloads` (new)

Publish every release's CLI binaries to a download host the project owns,
`https://dl.oidc-exchange.iamstan.dev`, in addition to GitHub Releases. Each release job signs the
binaries and a checksum manifest with Sigstore keyless signing (`cosign sign-blob`), so a consumer
can authenticate a download with one pinned identity and no GitHub account or `gh` CLI. The host is a
private S3 bucket behind CloudFront, defined with AWS CDK in a new `infra/downloads` workspace
package. `install.sh` prefers the new host and verifies the signature bundle when `cosign` is
present. This is PR 2 of 3 of the release-distribution series (PR 1 adopted Changesets; PR 3 added the
Versions page).

---

## Motivation

Binary authenticity today depends on GitHub: the checksum is a corruption check only, and
provenance is verifiable only with the `gh` CLI (`05-distribution.md`, Install script). A user
without `gh` gets an unauthenticated install with a warning, and the project cannot serve its
binaries if GitHub Releases is unreachable or rate-limited.

A project-owned host with a signature verifiable by the standalone `cosign` tool removes the `gh`
dependency, gives a stable documented URL for mirrors and air-gapped fetches, and keeps the
existing GitHub attestation as a second, independent proof. Keyless signing adds no long-lived
signing key to store or rotate; the verification identity is the release workflow itself.

---

## Affected spec pages

| Canonical page | Nature of change |
|---|---|
| [`.specs/bindings/specs/05-distribution.md`](../bindings/specs/05-distribution.md) | Artifacts table gains the download host and signature column; Install script, Release pipeline and Supply-chain gates describe the new job, signing and verification |
| `.specs/bindings/specs/06-download-host.md` (new) | The CDK-defined host: resources, layout, cache policy, upload role, operator runbook |
| [`.specs/architecture-principles.md`](../architecture-principles.md) | Monorepo layout lists `infra/downloads` |
| [`.specs/README.md`](../README.md) | Index the new page and this change spec |

---

## Proposed changes

### `.specs/bindings/specs/05-distribution.md` → Artifacts (Modify)

> | Binary `oidc-exchange` | linux x64/arm64, windows x64, macOS arm64 | GitHub Releases and `dl.oidc-exchange.iamstan.dev` (+ checksums) | Sigstore keyless signature bundle per artifact and for `SHA256SUMS`, plus the GitHub/Sigstore build attestation |

### `.specs/bindings/specs/05-distribution.md` → Release pipeline (Add)

> A `publish-downloads` job runs after `build-binaries` and `validate`. It downloads the binary
> artifacts, installs a pinned `cosign`, and for each artifact writes `<artifact>.sigstore.bundle`
> with `cosign sign-blob --bundle`. It composes `SHA256SUMS` from the per-artifact `.sha256`
> files and signs it as `SHA256SUMS.sigstore.bundle`. Before uploading, it verifies every bundle
> with `cosign verify-blob` against the expected identity (below) and fails the job on any
> mismatch. It then assumes the upload role through GitHub OIDC and writes the version prefix
> with a conditional create (`If-None-Match: *`), so an existing version is never overwritten.
> `latest.json` is written last, only after the version prefix is complete. The job alone adds
> `id-token: write` (Fulcio certificate and AWS role assumption); it holds `contents: read`.
> `create-release` does not depend on this job, so a download-host outage cannot block the GitHub
> release.

### `.specs/bindings/specs/05-distribution.md` → Supply-chain gates (Add)

> The signing identity is the certificate subject
> `https://github.com/antstanley/oidc-exchange/.github/workflows/release.yml@refs/tags/v<version>`
> with issuer `https://token.actions.githubusercontent.com`. `cosign` is installed at a literal
> exact stable version through a SHA-pinned action. Workflow policy rejects an unpinned or ranged
> cosign version and any `id-token: write` on `publish-downloads` beyond the allowlist entry.

### `.specs/bindings/specs/05-distribution.md` → Install script (Modify)

> `install.sh` resolves the version from `https://dl.oidc-exchange.iamstan.dev/latest.json` (or
> uses `--version`), downloads the artifact, its `.sha256` and `.sigstore.bundle` from
> `/releases/v<version>/`, and verifies in this order: (1) the checksum; (2) if `cosign` is on
> `PATH`, `cosign verify-blob --bundle` with the identity above, pinned to the version being
> installed; (3) otherwise, if `gh` is on `PATH`, the existing `gh attestation verify`; (4)
> otherwise it prints the existing checksum-only warning. A signature or attestation failure aborts
> the install. If the download host is unreachable or returns a non-2xx status for the version, the
> installer falls back to GitHub Releases for the same version and applies the same checks.

### `.specs/bindings/specs/06-download-host.md` (Add, new page)

> **Host.** `dl.oidc-exchange.iamstan.dev` is a CloudFront distribution in front of a private S3
> bucket reached through origin access control; the bucket blocks all public access and
> enforces TLS. An ACM certificate in `us-east-1` covers the hostname, and a Route 53 alias
> record points at the distribution.
>
> **Layout.**
> ```
> /latest.json                                     mutable, schema below
> /releases/v<version>/<artifact>                  immutable
> /releases/v<version>/<artifact>.sha256           immutable
> /releases/v<version>/<artifact>.sigstore.bundle  immutable
> /releases/v<version>/SHA256SUMS                  immutable
> /releases/v<version>/SHA256SUMS.sigstore.bundle  immutable
> ```
> Versioned objects are served `Cache-Control: public, max-age=31536000, immutable`;
> `latest.json` is served `max-age=60`. Responses carry HSTS and `X-Content-Type-Options: nosniff`.
> Bucket versioning is on and the bucket has `RemovalPolicy.RETAIN`.
>
> **Upload role.** A GitHub OIDC role trusts only `repo:antstanley/oidc-exchange` jobs in the
> `downloads` GitHub environment whose `job_workflow_ref` is `release.yml` at a `v*.*.*` tag. It can
> `s3:PutObject` under `releases/*` and `latest.json` and nothing else: no delete, no list outside
> its prefix, and no CloudFront or IAM access.
>
> **Definition.** The stack is the CDK app in `infra/downloads` (`DownloadsStack`). Deployment is
> an operator action (`pnpm --filter downloads cdk deploy`), never a CI step; CI runs `cdk synth`
> and the stack's assertion tests.

### `.specs/architecture-principles.md` → Monorepo layout (Modify)

> `infra/downloads` is a pnpm workspace package holding the CDK app for the download host. It
> depends on no other workspace package and ships nothing.

---

## Type changes

`latest.json` is a public contract for installers and mirrors. Folds into
`.specs/bindings/specs/canonical-types.schema.json` if one exists for distribution, otherwise
into the repo-wide schema.

```json
{
  "$comment": "Fragment for 2026-11-05-self_host_signed_binaries. Folds into the canonical schema on merge.",
  "$defs": {
    "DownloadsLatest": {
      "type": "object",
      "required": ["version", "publishedAt"],
      "additionalProperties": false,
      "properties": {
        "version": { "type": "string", "pattern": "^\\d+\\.\\d+\\.\\d+$" },
        "publishedAt": { "type": "string", "format": "date-time" }
      }
    }
  }
}
```

---

## Implementation notes

```
1. infra/downloads: new pnpm workspace package (add to pnpm-workspace.yaml; regenerate the three
   mirrored pnpm lockfiles). CDK app + DownloadsStack: S3 bucket (private, versioned, TLS-only,
   RETAIN), CloudFront distribution with OAC and a response-headers policy, ACM certificate,
   Route 53 alias, GitHub OIDC upload role. Context inputs: hostedZoneId, zoneName; env fixed to account 115019372186 / us-east-1; imports the existing GitHub OIDC provider.
   Assertion tests via aws-cdk-lib/assertions (no public bucket, role scope, cache headers).
2. release.yml: add publish-downloads (needs build-binaries, validate); install cosign pinned;
   sign, verify, conditional upload, latest.json last.
3. scripts/workflow-policy.mjs: add "release.yml:publish-downloads" -> {id-token} to
   ALLOWED_WRITE_PERMISSIONS; add a rule requiring an exact cosign version; add fixtures and
   bump the fixture counts in scripts/__tests__/workflow-policy.test.mjs.
4. install.sh and scripts/test-install.sh: new host, cosign path, gh path, checksum-only path,
   fallback to GitHub, tampered-bundle and wrong-identity cases.
5. Docs: apps/website installation page (not a spec page) and the create-release body (verify commands for cosign).
6. Out-of-band: operator deploys the stack, creates the `downloads` GitHub environment with a
   `v*.*.*` tag deployment rule, and adds the role ARN to the workflow.
```

---

## Merge plan

1. Apply each `Proposed changes` block to its canonical page; bump that page's `**Date:**`.
2. Create `.specs/bindings/specs/06-download-host.md` and index it in `.specs/README.md` and
   `.specs/bindings/README.md`.
3. Fold the `DownloadsLatest` `$def` into the canonical schema.
4. Flip this file's `**Status:**` to `Merged`, add `**Merged:** YYYY-MM-DD`, move it to
   `.specs/changes/merged/`, and update the `.specs/README.md` row.

---

## Assumptions and open questions

**Assumptions**

- The release workflow's GitHub OIDC token can obtain a Fulcio certificate through `cosign` without
  extra repository configuration, as `actions/attest-build-provenance` already does.
- S3 conditional writes (`If-None-Match: *`) are available to the AWS CLI version on the runner
  and S3 enforces them for `PutObject`.
- The stack deploys to the same AWS account and region as the blogwright docs deployment
  (`115019372186`, `us-east-1`; `config/production.jsonc`). Per that config the hosted zone for
  the docs domain lives in this account; the zone ID is supplied as CDK context.

**Decisions**

- *Signing.* **Sigstore keyless via `cosign sign-blob --bundle`.** No long-lived key to guard or
  rotate; the identity is the pinned release workflow, consistent with the existing attestation
  policy.
- *Hosting.* **S3 + CloudFront defined in CDK, in the docs account (`115019372186`, `us-east-1`).**
  Matches the requested tooling and reuses the account that already holds the zone and the
  GitHub OIDC provider for this repo; resource names carry an `oidc-exchange-downloads` prefix so
  they stay separate from the blogwright-managed `production-oidc-exchange-*` resources.
- *Mutability.* **Versioned prefixes are immutable; only `latest.json` is mutable.** A re-run of a
  release job fails the conditional create instead of replacing signed bytes.
- *Trust of GitHub.* **GitHub Releases stays published and stays the installer fallback.** Both
  sources are verified against the same identity, so the fallback adds no weaker path.
- *CDK deployment.* **Operator-run, not CI-run.** The deploy role would be far broader than the
  upload role, so it never exists in the release workflow.

**Open questions**

- Can the stack create the ACM validation records and the alias in the account's hosted zone, or
  does DNS stay a manual step as it is for the docs site apex (`config/production.jsonc`)?
- Does the account already have a GitHub OIDC identity provider (created by `blogwright bootstrap`)?
  The stack would import it rather than create a second one, which AWS rejects for the same URL.
- Should a missing `cosign` and missing `gh` fail the install, with an explicit
  `--allow-unauthenticated` flag, instead of the current checksum-only warning? This change keeps
  today's behaviour.
- Add musl or Intel-macOS targets, or keep the current four?
- Sign the Docker image with cosign too, or leave it on GHCR attestations only?
