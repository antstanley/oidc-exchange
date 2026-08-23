#!/usr/bin/env bash
set -euo pipefail

readonly REPO="antstanley/oidc-exchange"
readonly SIGNER_WORKFLOW="antstanley/oidc-exchange/.github/workflows/release.yml"
readonly BINARY_NAME="oidc-exchange"
readonly GH_VERIFY_TIMEOUT="30s"
VERSION=""

# Parse arguments
while [[ $# -gt 0 ]]; do
    case "$1" in
        --version)
            if [[ $# -lt 2 || -z "$2" ]]; then
                echo "Error: --version requires a release tag" >&2
                echo "Usage: install.sh [--version v1.2.3]" >&2
                exit 1
            fi
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

validate_version() {
    local version="$1"
    if [[ ! "$version" =~ ^v?[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z]+([.-][0-9A-Za-z]+)*)?$ ]]; then
        echo "Error: Invalid version '$version'. Expected vX.Y.Z or vX.Y.Z-prerelease." >&2
        exit 1
    fi
}

if [[ -n "$VERSION" ]]; then
    validate_version "$VERSION"
fi

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
    validate_version "$VERSION"
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

# Verify checksum. Missing checksum tools remain warn-and-continue until sibling task 07.
checksum_verified=false
echo "Verifying checksum..."
cd "$TMPDIR"
if command -v sha256sum &>/dev/null; then
    sha256sum -c "${BINARY_FILENAME}.sha256"
    checksum_verified=true
elif command -v shasum &>/dev/null; then
    shasum -a 256 -c "${BINARY_FILENAME}.sha256"
    checksum_verified=true
else
    echo "Warning: Neither sha256sum nor shasum found. Skipping checksum verification."
fi

# Verify provenance when GitHub CLI is available. The repository and signer
# workflow are constants, never values derived from installer input.
if command -v gh &>/dev/null; then
    echo "Verifying GitHub build provenance..."
    if command -v timeout &>/dev/null; then
        timeout "$GH_VERIFY_TIMEOUT" gh attestation verify             "${TMPDIR}/${BINARY_FILENAME}"             --repo "$REPO"             --signer-workflow "$SIGNER_WORKFLOW" >/dev/null
    else
        gh attestation verify             "${TMPDIR}/${BINARY_FILENAME}"             --repo "$REPO"             --signer-workflow "$SIGNER_WORKFLOW" >/dev/null
    fi
else
    if [[ "$checksum_verified" = true ]]; then
        echo "Warning: GitHub CLI not found; checksum verified corruption only, artifact provenance was not authenticated." >&2
    else
        echo "Warning: Neither checksum nor provenance authenticity was verified because sha256sum, shasum, and GitHub CLI are unavailable; continuing due to the current missing-tool limitation." >&2
    fi
fi

# Determine install directory
if [[ "$(id -u)" -eq 0 ]]; then
    INSTALL_DIR="/usr/local/bin"
else
    INSTALL_DIR="${HOME}/.local/bin"
    mkdir -p "$INSTALL_DIR"
fi

# Install
chmod +x "${TMPDIR}/${BINARY_FILENAME}"
mv "${TMPDIR}/${BINARY_FILENAME}" "${INSTALL_DIR}/${BINARY_NAME}"

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
