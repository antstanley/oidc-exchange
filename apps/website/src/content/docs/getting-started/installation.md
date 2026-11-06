---
title: Installation
description: Install oidc-exchange with a script, prebuilt binary, Docker, npm, pip, or a source build.
---

## Quick install (Linux / macOS)

```bash
curl -fsSL https://raw.githubusercontent.com/antstanley/oidc-exchange/main/install.sh | bash
```

The installer downloads from `dl.oidc-exchange.iamstan.dev` (falling back to GitHub Releases) and verifies the checksum, then:

1. if `cosign` is installed, the Sigstore signature, pinned to `antstanley/oidc-exchange` and the release workflow for that exact version;
2. otherwise, if the GitHub CLI (`gh`) is installed, the GitHub build provenance;
3. otherwise it prints an explicit warning and proceeds with checksum-only corruption detection; the artifact is not authenticated.

A failed signature or provenance check aborts the install. If the download host is unreachable, the installer falls back to GitHub Releases for the same version (the cosign check applies to the download host only).

To verify a download yourself, without the installer:

```bash
BASE=https://dl.oidc-exchange.iamstan.dev/releases/v0.5.1
curl -fsSLO "$BASE/oidc-exchange-linux-x64"
curl -fsSLO "$BASE/oidc-exchange-linux-x64.sigstore.bundle"
cosign verify-blob \
  --bundle oidc-exchange-linux-x64.sigstore.bundle \
  --certificate-identity https://github.com/antstanley/oidc-exchange/.github/workflows/release.yml@refs/tags/v0.5.1 \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  oidc-exchange-linux-x64
```

To install a specific version:

```bash
curl -fsSL https://raw.githubusercontent.com/antstanley/oidc-exchange/main/install.sh | bash -s -- --version v0.5.1
```

## Docker

```bash
docker pull ghcr.io/antstanley/oidc-exchange:latest
```

Or from Docker Hub:

```bash
docker pull antstanley80/oidc-exchange:latest
```

Run with a configuration file:

```bash
docker run -p 8080:8080 -v ./config:/app/config ghcr.io/antstanley/oidc-exchange:latest
```

## Node.js

```bash
npm install @oidc-exchange/node
```

See the [Node.js guide](/guides/nodejs) for framework-specific setup.

## Python

```bash
pip install oidc-exchange
```

See the [Python guide](/guides/python) for framework-specific setup.

## Prebuilt binaries

Download a binary from the project download host, `https://dl.oidc-exchange.iamstan.dev/releases/v<version>/<binary>` (the latest version is listed in [`latest.json`](https://dl.oidc-exchange.iamstan.dev/latest.json)). Each release also carries a `SHA256SUMS` file and a Sigstore `.sigstore.bundle` per binary. The same files are attached to [GitHub Releases](https://github.com/antstanley/oidc-exchange/releases):

| Platform | Binary |
|----------|--------|
| Linux x86_64 | `oidc-exchange-linux-x64` |
| Linux ARM64 | `oidc-exchange-linux-arm64` |
| macOS ARM64 | `oidc-exchange-darwin-arm64` |
| Windows x86_64 | `oidc-exchange-windows-x64.exe` |

## From source

Install a recent stable [Rust](https://rustup.rs/) toolchain. Continuous integration (CI) builds and tests use rustc 1.98. Build the binary with:

```bash
git clone https://github.com/antstanley/oidc-exchange.git
cd oidc-exchange
cargo build --release
```

The binary is at `target/release/oidc-exchange`.

When neither `cosign` nor the GitHub CLI is available, the installer reports whether checksum verification actually succeeded. If checksum tooling is also unavailable (neither `sha256sum` nor `shasum` is present), the installer aborts before downloading anything rather than installing an unverified binary. Whenever `cosign` (download host) or `gh` is present, a signature or provenance failure aborts installation.
