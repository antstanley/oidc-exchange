---
title: Installation
description: Install oidc-exchange with a script, prebuilt binary, Docker, npm, pip, or a source build.
---

## Quick install (Linux / macOS)

```bash
curl -fsSL https://dl.oidc-exchange.iamstan.dev/install.sh | bash
```

The installer downloads the binary, checksum and signature bundle from `dl.oidc-exchange.iamstan.dev` only. It verifies the SHA-256 checksum (using `sha256sum` or `shasum`; if neither is installed it aborts before downloading anything, and a checksum mismatch aborts the install), then:

1. if `cosign` is installed, the Sigstore signature, pinned to `antstanley/oidc-exchange` and the release workflow for that exact version;
2. otherwise it prints an explicit warning and proceeds with checksum-only corruption detection; the artifact is not authenticated.

A failed signature check aborts the install. The installer contacts the download host only, and no GitHub service: if it is unreachable the install fails rather than falling back to another source.

The binary is installed as `oidc-exchange` in `/usr/local/bin` when the installer runs as root, otherwise in `~/.local/bin` (created if missing). If that directory is not on your `PATH`, the installer prints the `export PATH=...` line to add.

The installer is itself signed. To check it before running it:

```bash
curl -fsSLO https://dl.oidc-exchange.iamstan.dev/install.sh
curl -fsSLO https://dl.oidc-exchange.iamstan.dev/install.sh.sigstore.bundle
cosign verify-blob \
  --bundle install.sh.sigstore.bundle \
  --certificate-identity https://github.com/antstanley/oidc-exchange/.github/workflows/release.yml@refs/tags/v0.6.0 \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  install.sh
```

To verify a download yourself, without the installer:

```bash
BASE=https://dl.oidc-exchange.iamstan.dev/releases/v0.6.0
curl -fsSLO "$BASE/oidc-exchange-linux-x64"
curl -fsSLO "$BASE/oidc-exchange-linux-x64.sigstore.bundle"
cosign verify-blob \
  --bundle oidc-exchange-linux-x64.sigstore.bundle \
  --certificate-identity https://github.com/antstanley/oidc-exchange/.github/workflows/release.yml@refs/tags/v0.6.0 \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  oidc-exchange-linux-x64
```

To install a specific version (the `v` prefix is optional, and pre-release versions such as `v1.0.0-rc.1` are accepted; the latest version is read from `latest.json`, which never points at a pre-release):

```bash
curl -fsSL https://dl.oidc-exchange.iamstan.dev/install.sh | bash -s -- --version v0.6.0
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

Install a recent stable [Rust](https://rustup.rs/) toolchain. Continuous integration (CI) uses the `stable` toolchain and the Docker image builds with `rust:1.99`. Build the binary with:

```bash
git clone https://github.com/antstanley/oidc-exchange.git
cd oidc-exchange
cargo build --release
```

The binary is at `target/release/oidc-exchange`.
