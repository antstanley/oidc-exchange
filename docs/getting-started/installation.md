---
title: Installation
description: Install oidc-exchange via one-line script, prebuilt binary, Docker, npm, pip, or from source.
---

## Quick Install (Linux / macOS)

```bash
curl -fsSL https://raw.githubusercontent.com/antstanley/oidc-exchange/main/install.sh | bash
```

The installer verifies the downloaded binary's checksum and, when the GitHub CLI (`gh`) is available, requires GitHub build provenance from `antstanley/oidc-exchange` and `.github/workflows/release.yml`. Without `gh`, it prints an explicit warning and proceeds with checksum-only corruption detection; the artifact is not authenticated.


To install a specific version:

```bash
curl -fsSL https://raw.githubusercontent.com/antstanley/oidc-exchange/main/install.sh | bash -s -- --version v1.0.0
```

## Docker

```bash
docker pull ghcr.io/antstanley/oidc-exchange:latest
```

Or from Docker Hub:

```bash
docker pull antstanley/oidc-exchange:latest
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

## Prebuilt Binaries

Download from [GitHub Releases](https://github.com/antstanley/oidc-exchange/releases):

| Platform | Binary |
|----------|--------|
| Linux x86_64 | `oidc-exchange-linux-x64` |
| Linux ARM64 | `oidc-exchange-linux-arm64` |
| macOS ARM64 | `oidc-exchange-darwin-arm64` |
| Windows x86_64 | `oidc-exchange-windows-x64.exe` |

## From Source

Requires [Rust 1.75+](https://rustup.rs/):

```bash
git clone https://github.com/antstanley/oidc-exchange.git
cd oidc-exchange
cargo build --release
```

The binary is at `target/release/oidc-exchange`.

When GitHub CLI is unavailable, the installer reports whether checksum verification actually succeeded. If checksum tooling is also unavailable, the current installer warns loudly that neither checksum nor provenance authenticity was verified and continues; fail-closed handling for that missing-tool case is tracked separately. Whenever `gh` is present, provenance failure aborts installation even when checksum tools are unavailable.
