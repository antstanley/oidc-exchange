#!/usr/bin/env bash
# install.sh: download host, cosign verification order and GitHub fallback.
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
INSTALLER="$ROOT/install.sh"
PASS=0
absent() { if grep -q "$@"; then echo "FAIL: unexpected match: $*" >&2; exit 1; fi; }
IDENTITY='https://github.com/antstanley/oidc-exchange/.github/workflows/release.yml@refs/tags/v1.2.3'

run_case() {
  local name=$1 cosign=$2 gh=$3 dl=$4 expect=$5
  local sandbox bin log home status
  sandbox=$(mktemp -d)
  bin="$sandbox/bin"; log="$sandbox/calls"; home="$sandbox/home"
  mkdir -p "$bin" "$home"; : > "$log"
  printf '#!/bin/sh\n[ "$1" = "-s" ] && printf Linux || printf x86_64\n' > "$bin/uname"
  printf '#!/bin/sh\nprintf 1000\n' > "$bin/id"
  cat > "$bin/curl" <<'EOS'
#!/bin/sh
printf 'curl %s\n' "$*" >> "$CALL_LOG"
out=; url=
while [ "$#" -gt 0 ]; do
  case "$1" in -o) out=$2; shift 2; continue;; -*) shift; continue;; *) url=$1; shift;; esac
done
if [ "$DL" = down ]; then case "$url" in https://dl.oidc-exchange.iamstan.dev/*) exit 22;; esac; fi
case "$out" in
  *.sha256) printf '%s  %s\n' "$(printf fixture-binary | /usr/bin/shasum -a 256 | cut -d' ' -f1)" "$(basename "${out%.sha256}")" > "$out" ;;
  '') printf '{"version":"1.2.3","publishedAt":"2026-11-05T00:00:00Z"}\n' ;;
  *) printf fixture-binary > "$out" ;;
esac
EOS
  printf '#!/bin/sh\nexec /usr/bin/shasum -a 256 "$@"\n' > "$bin/sha256sum"
  if [ "$cosign" != none ]; then
    cat > "$bin/cosign" <<'EOS'
#!/bin/sh
printf 'cosign %s\n' "$*" >> "$CALL_LOG"
[ "$COSIGN" = fail ] && exit 1
exit 0
EOS
  fi
  if [ "$gh" != none ]; then
    printf '#!/bin/sh\nprintf "gh %%s\\n" "$*" >> "$CALL_LOG"\n[ "$GH" = fail ] && exit 1\nexit 0\n' > "$bin/gh"
  fi
  chmod +x "$bin"/*
  set +e
  PATH="$bin:/usr/bin:/bin" HOME="$home" CALL_LOG="$log" COSIGN="$cosign" GH="$gh" DL="$dl" \
    bash "$INSTALLER" "${@:6}" >"$sandbox/out" 2>"$sandbox/err"
  status=$?
  set -e
  case "$expect" in
    ok)   [ "$status" -eq 0 ] && [ -f "$home/.local/bin/oidc-exchange" ] ;;
    fail) [ "$status" -ne 0 ] && [ ! -e "$home/.local/bin/oidc-exchange" ] ;;
  esac
  case "$name" in
    cosign-verifies)
      grep -Fq "cosign verify-blob --bundle" "$log"
      grep -Fq -- "--certificate-identity $IDENTITY --certificate-oidc-issuer https://token.actions.githubusercontent.com" "$log"
      grep -Fq 'releases/v1.2.3/oidc-exchange-linux-x64.sigstore.bundle' "$log"
      absent '^gh ' "$log" ;;
    cosign-preferred-over-gh) absent '^gh ' "$log" ;;
    cosign-failure-aborts) grep -Fq 'cosign verify-blob' "$log" ;;
    gh-when-no-cosign) grep -Fq 'gh attestation verify' "$log"; absent 'sigstore.bundle' "$log" ;;
    checksum-only) grep -Fq 'corruption only' "$sandbox/err" ;;
    github-fallback)
      grep -Fq 'github.com/antstanley/oidc-exchange/releases/download/v1.2.3/oidc-exchange-linux-x64' "$log"
      grep -Fq 'gh attestation verify' "$log"; absent 'cosign' "$log" ;;
    latest-from-download-host)
      grep -Fq 'dl.oidc-exchange.iamstan.dev/latest.json' "$log"
      grep -Fq 'releases/v1.2.3/oidc-exchange-linux-x64' "$log"
      absent 'api.github.com' "$log" ;;
  esac
  rm -rf "$sandbox"
  PASS=$((PASS + 1))
  printf 'ok %d - %s\n' "$PASS" "$name"
}

#        name                     cosign gh    dl   expect  args...
run_case cosign-verifies          ok     none  up   ok      --version v1.2.3
run_case cosign-preferred-over-gh ok     ok    up   ok      --version v1.2.3
run_case cosign-failure-aborts    fail   ok    up   fail    --version v1.2.3
run_case gh-when-no-cosign        none   ok    up   ok      --version v1.2.3
run_case checksum-only            none   none  up   ok      --version v1.2.3
run_case github-fallback          ok     ok    down ok      --version v1.2.3
run_case latest-from-download-host ok    none  up   ok

[ "$PASS" -eq 7 ]
printf '1..7\n'
