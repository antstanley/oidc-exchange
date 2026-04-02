#!/usr/bin/env bash
set -euo pipefail

REPO="antstanley/oidc-exchange"
BINARY_NAME="oidc-exchange"
VERSION=""

# Parse arguments
while [[ $# -gt 0 ]]; do
    case "$1" in
        --version)
            VERSION="$2"
            shift 2
            ;;
        *)
            echo "Unknown argument: $1"
            echo "Usage: install.sh [--version v1.2.3]"
            exit 1
            ;;
    esac
done

# Detect OS
OS="$(uname -s)"
case "$OS" in
    Linux)  OS_LABEL="linux" ;;
    Darwin) OS_LABEL="darwin" ;;
    *)
        echo "Error: Unsupported operating system: $OS"
        echo "Supported: Linux, macOS (Darwin)"
        exit 1
        ;;
esac

# Detect architecture
ARCH="$(uname -m)"
case "$ARCH" in
    x86_64)
        if [ "$OS_LABEL" = "darwin" ]; then
            echo "Error: macOS x86_64 (Intel) is not supported. Only Apple Silicon (arm64) is supported."
            exit 1
        fi
        ARCH_LABEL="x64"
        ;;
    aarch64|arm64)  ARCH_LABEL="arm64" ;;
    *)
        echo "Error: Unsupported architecture: $ARCH"
        echo "Supported: x86_64 (Linux), aarch64/arm64"
        exit 1
        ;;
esac

BINARY_FILENAME="${BINARY_NAME}-${OS_LABEL}-${ARCH_LABEL}"

# Resolve version
if [[ -z "$VERSION" ]]; then
    echo "Fetching latest version..."
    VERSION="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/')"
    if [[ -z "$VERSION" ]]; then
        echo "Error: Could not determine latest version from GitHub API"
        exit 1
    fi
fi

echo "Installing ${BINARY_NAME} ${VERSION} (${OS_LABEL}/${ARCH_LABEL})..."

DOWNLOAD_BASE="https://github.com/${REPO}/releases/download/${VERSION}"
BINARY_URL="${DOWNLOAD_BASE}/${BINARY_FILENAME}"
CHECKSUM_URL="${DOWNLOAD_BASE}/${BINARY_FILENAME}.sha256"

# Create temp directory
TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

# Download binary and checksum
echo "Downloading binary..."
curl -fsSL -o "${TMPDIR}/${BINARY_FILENAME}" "$BINARY_URL"

echo "Downloading checksum..."
curl -fsSL -o "${TMPDIR}/${BINARY_FILENAME}.sha256" "$CHECKSUM_URL"

# Verify checksum
echo "Verifying checksum..."
cd "$TMPDIR"
if command -v sha256sum &>/dev/null; then
    sha256sum -c "${BINARY_FILENAME}.sha256"
elif command -v shasum &>/dev/null; then
    shasum -a 256 -c "${BINARY_FILENAME}.sha256"
else
    echo "Warning: Neither sha256sum nor shasum found. Skipping checksum verification."
fi

# Determine install directory
if [[ "$(id -u)" -eq 0 ]]; then
    INSTALL_DIR="/usr/local/bin"
else
    INSTALL_DIR="${HOME}/.local/bin"
    mkdir -p "$INSTALL_DIR"
fi

# Install
chmod +x "${BINARY_FILENAME}"
mv "${BINARY_FILENAME}" "${INSTALL_DIR}/${BINARY_NAME}"

echo ""
echo "Installed ${BINARY_NAME} ${VERSION} to ${INSTALL_DIR}/${BINARY_NAME}"

# Check if install dir is in PATH
if ! echo "$PATH" | tr ':' '\n' | grep -qx "$INSTALL_DIR"; then
    echo ""
    echo "Warning: ${INSTALL_DIR} is not in your PATH."
    echo "Add it by running:"
    echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
    echo ""
    echo "To make this permanent, add the line above to your shell profile (~/.bashrc, ~/.zshrc, etc.)"
fi
