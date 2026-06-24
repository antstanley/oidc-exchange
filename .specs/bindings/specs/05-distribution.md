# Distribution

**Status:** Implemented · **Date:** 2026-06-24 · **Owner:** Ant Stanley · **Scope:** install.sh, Dockerfile, .github/workflows

How every artifact is built and shipped: the binary install script, the Docker image, the
language packages, and the tag-triggered release pipeline.

## Artifacts

| Artifact | Target | Channel |
|---|---|---|
| Binary `oidc-exchange` | linux x64/arm64, windows x64, macOS arm64 | GitHub Releases (+ checksums) |
| Docker image | `linux/amd64`, `linux/arm64` | ghcr.io and Docker Hub |
| `@oidc-exchange/node` (+ platform packages) | 4 napi targets | npm |
| `oidc-exchange` wheels | 4 abi3 platforms | PyPI |
| `@oidc-exchange/lambda` | TypeScript | npm |

## Install script (`install.sh`)

A bash installer for `antstanley/oidc-exchange`. It detects OS (`uname -s`) and arch
(`uname -m`), maps to a release asset name, downloads the binary and its SHA-256 checksum from
GitHub Releases, verifies the checksum, and installs to `/usr/local/bin` (root) or
`~/.local/bin` (non-root). It accepts a `--version`/positional pin and defaults to the latest
release; it requires `curl`/`wget` and `sha256sum`/`shasum`.

## Docker (`Dockerfile`)

Multi-stage: `rust:1.85-slim` builder (`cargo build --release --bin oidc-exchange`, with
`pkg-config`/`libssl-dev`) → `debian:bookworm-slim` runtime with `ca-certificates`/`curl`,
exposing 8080 and running the binary. Example Dockerfiles layer config onto the published
image.

## Release pipeline (`.github/workflows`)

**CI (`ci.yml`)** runs on push/PR: `lint` (`cargo fmt --check`, `cargo clippy -- -D
warnings`), `test` (`cargo nextest run --workspace`), `nodejs-test` (build the napi module,
vitest, lint/fmt), `python-test` (maturin build, pytest via uv).

**Release (`release.yml`)** triggers on a `v*.*.*` tag and runs: `validate` (extract version
from the tag), `build-binaries` (matrix per target, checksums), `build-docker` (buildx
multi-arch, push to both registries with `latest`/`vX.Y.Z`/`vX.Y`/`vX` tags), `build-nodejs` +
`publish-nodejs`, `build-python` + `publish-python`, and `create-release` (GitHub Release with
binaries and a generated changelog).

## Version parity

One version string must match across `Cargo.toml` `workspace.package.version`,
`bindings/nodejs/package.json`, and `bindings/python/pyproject.toml`. The `validate` job checks
this before building. Bumps are manual: edit the three files, commit, tag, push. npm and PyPI
use the bare `X.Y.Z`; GitHub and Docker use the `v`-prefixed tag.

## Assumptions and open questions

### Assumptions

- npm, PyPI, ghcr.io, and Docker Hub credentials are configured as repository secrets for the
  publish jobs.
- The git tag is the single source of truth for a release's version.

### Decisions

- *Tag-triggered unified release.* **A `v*.*.*` tag drives binaries, Docker, npm, and PyPI in
  one pipeline.** Atomic, consistent versioning across every artifact from one monorepo.
- *Checksum-verified install.* **`install.sh` verifies SHA-256 before installing.** Detects
  tampering or truncated downloads.

### Open questions

- Version bumps are manual across three manifests; a single-command bump (or a release-please
  style automation) is not yet in place.
