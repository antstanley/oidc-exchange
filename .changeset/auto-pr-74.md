---
"@oidc-exchange/node": minor
---

Serve the signed installer from the download host and drop GitHub fallbacks

GitHub storage limits mean older releases can't be guaranteed to stay there, so the download host becomes the only install source.

- `publish-downloads` signs `install.sh` and uploads it with its `.sigstore.bundle` to `releases/v<version>/` (immutable). `publish-latest` copies it to `/install.sh` at the root, before moving `latest.json`.
- The upload role gains `s3:PutObject` on `install.sh` and `install.sh.sigstore.bundle`. **The CDK stack must be redeployed before the next release.**
- `install.sh` drops the GitHub API and GitHub Releases fallbacks and fails closed if the host is unreachable. `gh attestation verify` stays as an optional verifier.
- README, quick-start, installation guide, release notes and specs now use `https://dl.oidc-exchange.iamstan.dev/install.sh`.

Stacked on #73. The root `/install.sh` only exists after the next release publishes it.
