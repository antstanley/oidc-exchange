---
title: Installation
description: Install oidc-exchange via one-line script, prebuilt binary, Docker, npm, pip, or from source.
---

## Quick Install (Linux / macOS)

```bash
curl -fsSL https://raw.githubusercontent.com/antstanley/oidc-exchange/main/install.sh | bash
```

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
