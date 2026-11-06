# Download host

**Status:** Implemented · **Date:** 2026-11-05 · **Owner:** Ant Stanley · **Scope:** infra/downloads

The project-owned host for release binaries, `https://dl.oidc-exchange.iamstan.dev`. Signing and
verification are specified in [05-distribution.md](05-distribution.md).

## Host

A CloudFront distribution in front of a private S3 bucket (`oidc-exchange-downloads-115019372186`)
reached through origin access control; the bucket blocks all public access and enforces TLS. An
ACM certificate in `us-east-1` covers the hostname and a Route 53 alias (A and AAAA) points at the
distribution. The stack creates the certificate validation records and the aliases in the
hosted zone supplied as CDK context: the existing `oidc-exchange.iamstan.dev` zone (the account has no `iamstan.dev` zone), so pass `zoneName=oidc-exchange.iamstan.dev`.

## Layout

```
/latest.json                                     mutable, schema below
/install.sh                                      mutable (copy of the latest stable release's installer)
/install.sh.sigstore.bundle                      mutable (matches /install.sh)
/releases/v<version>/<artifact>                  immutable
/releases/v<version>/<artifact>.sha256           immutable
/releases/v<version>/<artifact>.sigstore.bundle  immutable
/releases/v<version>/install.sh                  immutable
/releases/v<version>/install.sh.sigstore.bundle  immutable
/releases/v<version>/SHA256SUMS                  immutable
/releases/v<version>/SHA256SUMS.sigstore.bundle  immutable
```

Versioned objects are served `Cache-Control: public, max-age=31536000, immutable`; `latest.json`,
`install.sh` and its bundle are served `max-age=60`. Responses carry HSTS and `X-Content-Type-Options: nosniff`. Bucket
versioning is on and the bucket has `RemovalPolicy.RETAIN`.

`latest.json` is `{"version": "X.Y.Z", "publishedAt": "<date-time>"}` and nothing else; it is a
public contract for installers and mirrors.

## Upload role

`oidc-exchange-downloads-upload` trusts the account's existing GitHub OIDC provider only for
`repo:antstanley/oidc-exchange:environment:downloads` jobs whose `job_workflow_ref` is `release.yml`
at a `v*.*.*` tag. It can `s3:PutObject` under `releases/*` and on `latest.json`, `install.sh` and `install.sh.sigstore.bundle` and nothing else:
no delete, no list, no CloudFront or IAM access.

## Definition and operations

The stack is the CDK app in `infra/downloads` (`DownloadsStack`). Deployment is an operator action,
never a CI step; CI runs the stack's assertion tests (`pnpm --filter downloads test`).

```bash
pnpm --filter downloads exec cdk deploy -c hostedZoneId=<zone-id> -c zoneName=oidc-exchange.iamstan.dev
```

After the first deploy the operator creates the `downloads` GitHub environment with a deployment
rule restricted to `v*.*.*` tags.

## Assumptions and decisions

- The GitHub OIDC provider already exists in the account (created by `blogwright bootstrap`) and is
  imported, not created; AWS rejects a second provider for the same URL.
- The `job_workflow_ref` claim is evaluated as an IAM condition key on the GitHub provider. If AWS
  does not support that key, the condition fails closed (no role assumption) rather than open; the
  first release then surfaces it and the condition is replaced by a `sub` claim customization.
