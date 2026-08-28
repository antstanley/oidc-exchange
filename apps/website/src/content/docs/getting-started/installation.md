---
title: Installation
description: Install oidc-exchange via one-line script, prebuilt binary, Docker, npm, pip, or from source.
---

## Quick install (Linux / macOS)

```bash
curl -fsSL https://raw.githubusercontent.com/antstanley/oidc-exchange/main/install.sh | bash
```

The installer verifies the downloaded binary's checksum and, when the GitHub CLI (`gh`) is available, requires GitHub build provenance from `antstanley/oidc-exchange` and `.github/workflows/release.yml`. Without `gh`, it prints an explicit warning and proceeds with checksum-only corruption detection; the artifact is not authenticated.

To install a specific version:

```bash
curl -fsSL https://raw.githubusercontent.com/antstanley/oidc-exchange/main/install.sh | bash -s -- --version v0.4.0
```

## Docker

```bash
docker pull ghcr.io/antstanley/oidc-exchange:latest
```

Or from Docker Hub:

```bash
docker pull antstanley80/oidc-exchange:latest
```

Run with a config file:

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

Download from [GitHub Releases](https://github.com/antstanley/oidc-exchange/releases):

| Platform | Binary |
|----------|--------|
| Linux x86_64 | `oidc-exchange-linux-x64` |
| Linux ARM64 | `oidc-exchange-linux-arm64` |
| macOS ARM64 | `oidc-exchange-darwin-arm64` |
| Windows x86_64 | `oidc-exchange-windows-x64.exe` |

## From source

Requires a recent stable [Rust](https://rustup.rs/) toolchain (CI builds and tests on rustc 1.98):

```bash
git clone https://github.com/antstanley/oidc-exchange.git
cd oidc-exchange
cargo build --release
```

The binary is at `target/release/oidc-exchange`.

When GitHub CLI is unavailable, the installer reports whether checksum verification actually succeeded. If checksum tooling is also unavailable (neither `sha256sum` nor `shasum` is present), the installer aborts before downloading anything rather than installing an unverified binary. Whenever `gh` is present, provenance failure aborts installation.
