#!/usr/bin/env bash
set -euo pipefail

readonly REPO="antstanley/oidc-exchange"
readonly SIGNER_WORKFLOW="antstanley/oidc-exchange/.github/workflows/release.yml"
readonly BINARY_NAME="oidc-exchange"
readonly DOWNLOAD_HOST="https://dl.oidc-exchange.iamstan.dev"
readonly SIGNER_ISSUER="https://token.actions.githubusercontent.com"
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

# Select a checksum verifier before making any network requests.
if command -v sha256sum >/dev/null 2>&1; then
    CHECKSUM_VERIFIER="sha256sum"
elif command -v shasum >/dev/null 2>&1; then
    CHECKSUM_VERIFIER="shasum"
else
    echo "Error: Neither sha256sum nor shasum found; cannot verify checksum." >&2
    exit 1
fi

# Resolve the latest version from the project download host.
if [[ -z "$VERSION" ]]; then
    echo "Fetching latest version..."
    VERSION="$(curl -fsSL "${DOWNLOAD_HOST}/latest.json" 2>/dev/null | grep '"version"' | sed -E 's/.*"version": *"([^"]+)".*/v\1/' || true)"
    if [[ -z "$VERSION" ]]; then
        echo "Error: Could not determine latest version from ${DOWNLOAD_HOST}" >&2
        exit 1
    fi
    validate_version "$VERSION"
fi
# Release tags and download-host prefixes are always v-prefixed.
[[ "$VERSION" == v* ]] || VERSION="v${VERSION}"

echo "Installing ${BINARY_NAME} ${VERSION} (${OS_LABEL}/${ARCH_LABEL})..."

DOWNLOAD_BASE="${DOWNLOAD_HOST}/releases/${VERSION}"

# Create temp directory
TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

# Download binary and checksum from the project download host. There is no other
# source: if the host is unreachable the install fails rather than trusting a mirror.
echo "Downloading binary and checksum..."
BASE="$DOWNLOAD_BASE"
if ! curl -fsSL -o "${TMPDIR}/${BINARY_FILENAME}" "${BASE}/${BINARY_FILENAME}" ||
    ! curl -fsSL -o "${TMPDIR}/${BINARY_FILENAME}.sha256" "${BASE}/${BINARY_FILENAME}.sha256"; then
    echo "Error: could not download ${BINARY_FILENAME} ${VERSION} from ${BASE}" >&2
    exit 1
fi

# Verify checksum. A missing checksum tool already failed the install closed above.
checksum_verified=false
echo "Verifying checksum..."
cd "$TMPDIR"
if [[ "$CHECKSUM_VERIFIER" == "sha256sum" ]]; then
    sha256sum -c "${BINARY_FILENAME}.sha256"
    checksum_verified=true
else
    shasum -a 256 -c "${BINARY_FILENAME}.sha256"
    checksum_verified=true
fi

run_with_timeout() {
    if command -v timeout &>/dev/null; then
        timeout "$GH_VERIFY_TIMEOUT" "$@"
    else
        "$@"
    fi
}

# Authenticate the artifact. The repository, signer workflow and OIDC issuer are
# constants, never values derived from installer input. Order: Sigstore signature
# (cosign), then the optional GitHub build attestation (gh), then a loud
# checksum-only warning.
authenticated=false
if command -v cosign &>/dev/null; then
    echo "Downloading signature bundle..."
    curl -fsSL -o "${TMPDIR}/${BINARY_FILENAME}.sigstore.bundle" "${BASE}/${BINARY_FILENAME}.sigstore.bundle"
    echo "Verifying Sigstore signature..."
    run_with_timeout cosign verify-blob \
        --bundle "${TMPDIR}/${BINARY_FILENAME}.sigstore.bundle" \
        --certificate-identity "https://github.com/${REPO}/.github/workflows/release.yml@refs/tags/${VERSION}" \
        --certificate-oidc-issuer "$SIGNER_ISSUER" \
        "${TMPDIR}/${BINARY_FILENAME}" >/dev/null
    authenticated=true
elif command -v gh &>/dev/null; then
    echo "Verifying GitHub build provenance..."
    run_with_timeout gh attestation verify "${TMPDIR}/${BINARY_FILENAME}" \
        --repo "$REPO" \
        --signer-workflow "$SIGNER_WORKFLOW" >/dev/null
    authenticated=true
fi

if [[ "$authenticated" != true ]]; then
    if [[ "$checksum_verified" = true ]]; then
        echo "Warning: neither cosign nor GitHub CLI found; checksum verified corruption only, artifact provenance was not authenticated." >&2
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
