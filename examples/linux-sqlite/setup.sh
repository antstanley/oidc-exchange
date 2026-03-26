#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

echo "Setting up oidc-exchange SQLite example..."

# Create data directories
mkdir -p data lmdb keys

# Generate signing key if it doesn't exist
if [ ! -f keys/signing-key.pem ]; then
    echo "Generating Ed25519 signing key..."
    openssl genpkey -algorithm ed25519 -out keys/signing-key.pem
    echo "Key generated at keys/signing-key.pem"
fi

# Copy config files to config/ in working directory
mkdir -p config
cp "$SCRIPT_DIR/config/sqlite-only.toml" config/
cp "$SCRIPT_DIR/config/sqlite-lmdb.toml" config/

echo ""
echo "Setup complete. Run with:"
echo ""
echo "  SQLite only:"
echo "    OIDC_EXCHANGE_ENV=sqlite-only GOOGLE_CLIENT_ID=xxx GOOGLE_CLIENT_SECRET=yyy ./target/release/oidc-exchange"
echo ""
echo "  SQLite + LMDB:"
echo "    OIDC_EXCHANGE_ENV=sqlite-lmdb GOOGLE_CLIENT_ID=xxx GOOGLE_CLIENT_SECRET=yyy ./target/release/oidc-exchange"
echo ""
