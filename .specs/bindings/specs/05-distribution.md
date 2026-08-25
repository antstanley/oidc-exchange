# Distribution

**Status:** Implemented · **Date:** 2026-08-24 · **Owner:** Ant Stanley · **Scope:** install.sh, Dockerfile, .github/workflows

How every artifact is built, authenticated, and shipped.

## Artifacts

| Artifact | Target | Channel | Provenance |
|---|---|---|---|
| Binary `oidc-exchange` | linux x64/arm64, windows x64, macOS arm64 | GitHub Releases (+ checksums) | GitHub/Sigstore build attestation |
| Docker image | `linux/amd64`, `linux/arm64` | ghcr.io and Docker Hub | GHCR build attestations for each platform digest and the final manifest digest |
| `@oidc-exchange/node` + 4 platform packages (`@oidc-exchange/{linux-x64-gnu,linux-arm64-gnu,win32-x64-msvc,darwin-arm64}`) | 4 napi targets | npm (OIDC trusted publishing) | npm `--provenance` |
| `oidc-exchange` wheels | 4 abi3 platforms | PyPI (OIDC trusted publishing) | PyPI attestations |
| `@oidc-exchange/lambda` | TypeScript | npm (OIDC trusted publishing) | npm `--provenance` |

Every channel carries a claim binding an artifact digest to this repository, workflow, and tagged
revision. Binary checksums are corruption checks, not authenticity claims. Container build
provenance is attached in GHCR: each native build attests its immutable platform digest, the
manifest job composes exactly those digests, resolves the immutable multi-arch digest, and attests
that final digest. The workflow also copies the manifest to Docker Hub, but does not sign it there;
consumer-side `gh attestation verify oci://...` is therefore documented only for GHCR. Registry
signing and build provenance are distinct controls; this pipeline implements the latter.

## Install script (`install.sh`)

The Bash installer is fixed to repository `antstanley/oidc-exchange` and signer workflow
`antstanley/oidc-exchange/.github/workflows/release.yml`. It maps the detected OS and architecture
to a release asset. `--version` accepts only
`^v?[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z]+([.-][0-9A-Za-z]+)*)?$`; malformed or missing
operands fail before a request or URL construction. After downloading the binary and SHA-256
sidecar from GitHub Releases, it verifies the checksum. Verification is mandatory: if neither
checksum utility is present the installer prints the missing dependency and exits non-zero
**before** any download — there is no path through the script that installs an unverified
binary. If `gh` is present, installation also requires:

```bash
gh attestation verify <downloaded-binary> \
  --repo antstanley/oidc-exchange \
  --signer-workflow antstanley/oidc-exchange/.github/workflows/release.yml
```

A failed or timed-out provenance check stops installation. Without `gh`, the installer loudly says
that checksum verification detects corruption only and does not authenticate the artifact. It
installs to `/usr/local/bin` for root or `~/.local/bin` otherwise.

## Docker (`Dockerfile`)

A multi-stage Rust build produces the runtime binary in a minimal Debian image. Release builds run
natively for `linux/amd64` and `linux/arm64`, push each GHCR image by digest, and compose the final
manifest from those immutable subjects.

## Release pipeline (`.github/workflows`)

**CI (`ci.yml`)** runs format, clippy, nextest, Node native build/tests/lint/format/typecheck,
Python maturin/pytest/Ruff/Pyright, web-app checks, the three-graph advisory wrapper, and the
resolved signing-path policy. Every job has job-level permissions and no CI job has write scope.

**Release (`release.yml`)** starts with the same blocking dependency and signing-path policy.
There is no workflow-level permission grant. Every job declares only what it needs: checkout/build
jobs use `contents: read` with `persist-credentials: false`; image jobs add `packages: write`;
binary, per-platform image, and final-manifest attestation producers alone add `id-token: write`
and `attestations: write`; `create-release` alone has `contents: write`; npm/PyPI publishers alone
have `id-token: write` for trusted publishing. The binary attestation consumes the exact generated
`.sha256` subject checksum. Platform image attestations consume the build step digest. The manifest
attestation consumes the digest resolved after composing the two platform subjects.

`build-nodejs` installs with exact pnpm 11.9.0 and a frozen lockfile, then executes the locked napi
CLI offline. A separate read-only `validate-npm-package` job runs locked `publint` and
`@arethetypeswrong/cli`, distributes artifacts, and proves all four platform binaries exist before
uploading a validated package tree. Only then does `publish-npm` receive OIDC identity and stage
platform, root Node, and Lambda packages with provenance and ignored lifecycle scripts. Lambda uses
its own committed lockfile and a frozen, `--ignore-scripts` install. No publishing job resolves
`@latest`, runs `npx --yes`, or bypasses a lockfile.

Python builds four `cp310-abi3` wheels plus an sdist with the SHA-pinned maturin action and publishes
through PyPI Trusted Publishing. The binding uses `pyo3 0.29.2` with `extension-module` and
`abi3-py310`; the obsolete pyo3 advisory exceptions have been removed.

## Supply-chain gates

- **Pinning and lockfiles.** Every action is a full commit SHA. Executed tools are locked or exact:
  pnpm 11.9.0, cargo-deny 0.19.0, pip-audit 2.9.0, maturin 1.9.4, and cross 0.2.5. CI/release installs are frozen; publish-path
  installs use `--ignore-scripts`. `minimumReleaseAge` is explicit in `pnpm-workspace.yaml`.
- **Least privilege.** Permissions are per job. Checkout jobs explicitly retain only
  `contents: read`, disable persisted credentials when they do not push, and cannot inherit an
  omitted workflow-level write scope.
- **Three dependency graphs.** `scripts/run-advisory-scans.mjs` scans committed `Cargo.lock`, all
  owned pnpm lockfiles recursively, and the nonempty frozen build export of
  `bindings/python/uv.lock` (`maturin==1.9.4` plus its conditional `tomli==2.4.1` dependency). The production runtime export is separately empty. Unknown and expired high-severity findings fail; exact unexpired
  exceptions pass only when ecosystem, advisory, package, version/range, rationale, owner, expiry,
  and review date match. Cargo unmaintained and yanked findings warn. Scanner, database, registry,
  malformed-output, version, and frozen-export failures exit separately as tool failure rather
  than being reported as clean. `config/advisory-policy.json` is the exception inventory.
- **Signing paths.** `config/signing-path-policy.json` derives roots from the adapters source and
  evaluates locked Cargo metadata in `workspace-all-targets` and `linux-release-target` modes.
  Pre-release protected packages fail unless the exact mode/package/version/path exception is
  present, owned, reasoned, and unexpired. Fourteen inventory entries (seven per mode, all exercised; RSA public-key verification is shipped while private-key construction is test-only) expire
  2026-09-15; drift in a path, version, feature graph, or protected package fails closed.

## Version parity

One version must match across the workspace Cargo manifest, Node package, and Python project. The
tag-triggered `validate` job checks parity before builds.

## Assumptions and decisions

- GitHub-hosted release runners can mint OIDC identities and reach Sigstore and registries.
- Binary and image authenticity uses GitHub build provenance; checksums remain corruption fallback.
- npm and PyPI use trusted publishing, not stored publication tokens.
- The Docker Hub copy is not claimed to carry the GHCR attestation or a registry signature.
- Advisory exceptions are exact, dated policy records rather than blanket scanner ignores.
- Signing-path detection landed without an unrelated cryptographic dependency upgrade; the exact
  temporary RC exceptions must be removed, replaced, or deliberately reviewed by 2026-09-15.

### Open questions

- Version bumps remain manually coordinated across manifests.
- Whether Docker Hub should gain an independent registry-signing mechanism is future work.


## Runtime parity update

One version string must match across `Cargo.toml` `workspace.package.version`,
`bindings/nodejs/package.json`, and `bindings/python/pyproject.toml`. The `validate` job
checks this before building. Bumps are manual: edit the three files, commit, tag, push. npm
and PyPI use the bare `X.Y.Z`; GitHub and Docker use the `v`-prefixed tag. Because the three
artifacts share one version, a breaking change to the FFI surface bumps all of them together
