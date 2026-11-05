---
"@oidc-exchange/node": patch
---

Treat a missing latest.json as absent when the bucket answers 403

`publish-latest` failed on the v0.5.0 release: the first read of `latest.json` returned **403**, not 404, because the bucket has no `s3:ListBucket` for CloudFront and S3 answers 403 for a missing object (`releases/v9.9.9/x`, which does not exist, also returns 403). The job treated only 404 as "no latest yet" and failed closed.

Fix: 403/404 on `latest.json` count as absent only when this version's `SHA256SUMS` is readable from the same host (so a real outage or access problem still fails the job). Any other status still fails. Spec updated.

Validation: `pnpm test:supply-chain` (54 pass), workflow YAML parses, and the status/probe decision table was exercised offline. The job itself runs only on a real tag; v0.5.0's `latest.json` needs to be published separately (the tagged workflow is the old one).
