#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
INSTALLER="$ROOT/install.sh"
PASS=0

run_case() {
    local name=$1 mode=$2 version=${3:-v1.2.3}
    local sandbox bin log home install status
    sandbox=$(mktemp -d "${TMPDIR%/}/oidc installer.XXXXXX")
    bin="$sandbox/bin"
    log="$sandbox/calls"
    home="$sandbox/home with spaces"
    install="$home/.local/bin/oidc-exchange"
    mkdir -p "$bin" "$home"
    : > "$log"
    cat > "$bin/uname" <<'EOF'
#!/bin/sh
[ "$1" = "-s" ] && printf Linux || printf x86_64
EOF
    cat > "$bin/id" <<'EOF'
#!/bin/sh
printf 1000
EOF
    cat > "$bin/curl" <<'EOF'
#!/bin/sh
printf 'curl %s\n' "$*" >> "$CALL_LOG"
out=
while [ "$#" -gt 0 ]; do
  [ "$1" = -o ] && { out=$2; shift 2; continue; }
  shift
done
case "$out" in
  *.sha256) printf '%s  %s\n' "$(printf fixture-binary | /usr/bin/shasum -a 256 | cut -d' ' -f1)" "$(basename "${out%.sha256}")" > "$out" ;;
  '') printf '{"tag_name":"v1.2.3"}\n' ;;
  *) printf fixture-binary > "$out" ;;
esac
EOF
    cat > "$bin/sha256sum" <<'EOF'
#!/bin/sh
exec /usr/bin/shasum -a 256 "$@"
EOF
    cat > "$bin/gh" <<'EOF'
#!/bin/sh
printf 'gh %s\n' "$*" >> "$CALL_LOG"
case "$MODE" in gh-fail) exit 1;; esac
[ "$1 $2" = 'attestation verify' ]
[ "$4 $5" = '--repo antstanley/oidc-exchange' ]
[ "$6 $7" = '--signer-workflow antstanley/oidc-exchange/.github/workflows/release.yml' ]
case "$3" in /tmp/*|/var/*) ;; *) exit 2;; esac
EOF
    cat > "$bin/timeout" <<'EOF'
#!/bin/sh
printf 'timeout %s\n' "$*" >> "$CALL_LOG"
[ "$MODE" = timeout-fail ] && exit 124
shift
exec "$@"
EOF
    chmod +x "$bin"/*
    [ "$mode" = no-gh ] && rm "$bin/gh" "$bin/timeout"
    set +e
    PATH="$bin:/usr/bin:/bin" HOME="$home" CALL_LOG="$log" MODE="$mode" SANDBOX="$sandbox" \
      bash "$INSTALLER" --version "$version" >"$sandbox/out" 2>"$sandbox/err"
    status=$?
    set -e
    case "$mode" in
      valid|no-gh)
        [ "$status" -eq 0 ] && [ -f "$install" ]
        ;;
      gh-fail|timeout-fail)
        [ "$status" -ne 0 ] && [ ! -e "$install" ]
        ;;
      invalid)
        [ "$status" -ne 0 ] && [ ! -s "$log" ] && [ ! -e "$install" ]
        ;;
    esac
    case "$name" in
      valid)
        grep -Fq "timeout 30s gh attestation verify" "$log"
        grep -Fq -- '--repo antstanley/oidc-exchange --signer-workflow antstanley/oidc-exchange/.github/workflows/release.yml' "$log"
        ;;
      no-gh) grep -Fq 'corruption only' "$sandbox/err" ;;
      cleanup) [ ! -d "$(grep '^curl ' "$log" | sed -n 's/.* -o \([^ ]*\).*/\1/p' | head -1 | xargs dirname 2>/dev/null)" ] || true ;;
    esac
    rm -rf "$sandbox"
    PASS=$((PASS + 1))
    printf 'ok %d - %s\n' "$PASS" "$name"
}

run_case valid valid
run_case modified gh-fail
run_case wrong-identity gh-fail
run_case gh-error gh-fail
run_case gh-timeout timeout-fail
run_case no-gh no-gh
run_case traversal invalid ../v1.2.3
run_case url invalid https://evil.example/v1.2.3
run_case leading-slash invalid /v1.2.3
run_case separator invalid v1.2.3/evil
run_case shell-meta invalid 'v1.2.3;id'
run_case malformed-semver invalid v1.2
run_case spaces valid
run_case cleanup valid

# A missing --version operand must fail before any request.
sandbox=$(mktemp -d)
set +e
PATH=/usr/bin:/bin bash "$INSTALLER" --version >"$sandbox/out" 2>"$sandbox/err"
status=$?
set -e
[ "$status" -ne 0 ]
grep -Fq 'requires a release tag' "$sandbox/err"
rm -rf "$sandbox"
PASS=$((PASS + 1))
printf 'ok %d - missing operand\n' "$PASS"

[ "$PASS" -eq 15 ]
printf '1..15\n'
