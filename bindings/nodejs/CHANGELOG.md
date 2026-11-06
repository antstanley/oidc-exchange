# @oidc-exchange/node

## 0.5.1

### Patch Changes

- [#71](https://github.com/antstanley/oidc-exchange/pull/71) [`fc6dcbb`](https://github.com/antstanley/oidc-exchange/commit/fc6dcbb2387549967fdf073329a85dbd7323f491) Thanks [@antstanley](https://github.com/antstanley)! - Treat a missing latest.json as absent when the bucket answers 403

  `publish-latest` failed on the v0.5.0 release: the first read of `latest.json` returned **403**, not 404, because the bucket has no `s3:ListBucket` for CloudFront and S3 answers 403 for a missing object (`releases/v9.9.9/x`, which does not exist, also returns 403). The job treated only 404 as "no latest yet" and failed closed.

  Fix: 403/404 on `latest.json` count as absent only when this version's `SHA256SUMS` is readable from the same host (so a real outage or access problem still fails the job). Any other status still fails. Spec updated.

  Validation: `pnpm test:supply-chain` (54 pass), workflow YAML parses, and the status/probe decision table was exercised offline. The job itself runs only on a real tag; v0.5.0's `latest.json` needs to be published separately (the tagged workflow is the old one).

## 0.5.0

### Minor Changes

- [#69](https://github.com/antstanley/oidc-exchange/pull/69) [`b7a6f89`](https://github.com/antstanley/oidc-exchange/commit/b7a6f89fafee971e0c52aaf3d2bb569f2d72806f) Thanks [@antstanley](https://github.com/antstanley)! - Session listing, self-hosted signed binaries, and a Python 3.11 floor.

  **Features**

  - List a user's sessions with `GET /internal/users/{id}/sessions`: live sessions only, one per refresh-token family, newest first, capped at 50, each with an opaque stable `ses_` id. Callers of the exchange flow can supply a provider-neutral `SessionLocation`, carried forward on rotation and persisted by all five session adapters (DynamoDB, Postgres, SQLite, LMDB, Valkey). An opt-in `[session.location]` header mapping reads client location from trusted proxies only ([#63](https://github.com/antstanley/oidc-exchange/issues/63)).
  - Release binaries are now published to `https://dl.oidc-exchange.iamstan.dev` with Sigstore keyless signatures (`cosign`) for each artifact and `SHA256SUMS`, alongside GitHub Releases. `install.sh` downloads from the new host and verifies the checksum, then the cosign signature (or the GitHub attestation via `gh`), falling back to GitHub Releases if the host is unavailable ([#65](https://github.com/antstanley/oidc-exchange/issues/65)).
  - Releases are now versioned with Changesets, with a Versions page on the docs site, and changesets are generated from PR titles automatically ([#46](https://github.com/antstanley/oidc-exchange/issues/46), [#64](https://github.com/antstanley/oidc-exchange/issues/64), [#66](https://github.com/antstanley/oidc-exchange/issues/66)).

  **Breaking**

  - Python 3.10 is no longer supported. The package requires Python 3.11 or later, wheels are now `cp311-abi3`, and 3.14 and 3.15 are classified as supported ([#60](https://github.com/antstanley/oidc-exchange/issues/60)).

  **Maintenance**

  - Dependency and toolchain updates: pnpm 12, Node 26 LTS, heed 0.23, SvelteKit 3, TypeScript 7.1 beta, maturin and pinned actions, `cargo-deny` 0.20.2 and `pip-audit` 2.10.1. The expired `rsa` advisory exception is retired and `bincode` is dropped ([#54](https://github.com/antstanley/oidc-exchange/issues/54)–[#59](https://github.com/antstanley/oidc-exchange/issues/59), [#62](https://github.com/antstanley/oidc-exchange/issues/62)).
