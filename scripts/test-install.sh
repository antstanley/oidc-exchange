#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
INSTALLER="${ROOT}/install.sh"
TEST_TMPDIR="$(mktemp -d)"
trap 'rm -rf "${TEST_TMPDIR}"' EXIT

fail() {
    printf 'FAIL: %s\n' "$*" >&2
    exit 1
}

run_case() {
    local name="$1"
    local verifiers="$2"
    local checksum_status="$3"
    local expected_status="$4"
    local case_dir="${TEST_TMPDIR}/${name}"
    local bin_dir="${case_dir}/bin"
    local home_dir="${case_dir}/home"
    local log="${case_dir}/output"

    mkdir -p "${bin_dir}" "${home_dir}"

    cat > "${bin_dir}/uname" <<'EOF'
#!/bin/bash
if [[ "$1" == "-s" ]]; then
    printf 'Linux\n'
else
    printf 'x86_64\n'
fi
EOF
    cat > "${bin_dir}/id" <<'EOF'
#!/bin/bash
printf '1000\n'
EOF
    cat > "${bin_dir}/mktemp" <<'EOF'
#!/bin/bash
mkdir -p "${INSTALLER_TEST_TMPDIR}"
printf '%s\n' "${INSTALLER_TEST_TMPDIR}"
EOF
    cat > "${bin_dir}/curl" <<'EOF'
#!/bin/bash
set -euo pipefail
if [[ "$1" == "-fsSL" && "$2" == "-o" ]]; then
    printf 'curl\n' >> "${INSTALLER_TEST_EVENTS}"
    output="$3"
    url="$4"
    case "$url" in
        *.sha256) printf 'fixture-binary  oidc-exchange-linux-x64\n' > "$output" ;;
        *) printf 'fixture-binary\n' > "$output" ;;
    esac
    exit 0
fi
exit 2
EOF
    cat > "${bin_dir}/chmod" <<'EOF'
#!/bin/bash
printf 'chmod\n' >> "${INSTALLER_TEST_EVENTS}"
exit 0
EOF
    cat > "${bin_dir}/rm" <<'EOF'
#!/bin/bash
exit 0
EOF
    cat > "${bin_dir}/mv" <<'EOF'
#!/bin/bash
printf 'mv\n' >> "${INSTALLER_TEST_EVENTS}"
exit 0
EOF

    if [[ "$verifiers" == "none" ]]; then
        cat > "${case_dir}/bash-env" <<'EOF'
command() {
    if [[ "$1" == "-v" && ( "$2" == "sha256sum" || "$2" == "shasum" ) ]]; then
        return 1
    fi
    builtin command "$@"
}
EOF
    fi
    if [[ "$verifiers" == "sha256sum" || "$verifiers" == "both" ]]; then
        cat > "${bin_dir}/sha256sum" <<EOF
#!/bin/bash
printf 'sha256sum\n' >> "\${INSTALLER_TEST_EVENTS}"
exit ${checksum_status}
EOF
    fi
    if [[ "$verifiers" == "shasum" || "$verifiers" == "both" ]]; then
        cat > "${bin_dir}/shasum" <<EOF
#!/bin/bash
printf 'shasum\n' >> "\${INSTALLER_TEST_EVENTS}"
exit ${checksum_status}
EOF
    fi

    chmod +x "${bin_dir}"/*

    set +e
    PATH="${bin_dir}:/usr/bin:/bin" \
        INSTALLER_TEST_EVENTS="${case_dir}/events" \
        INSTALLER_TEST_TMPDIR="${case_dir}/installer-tmp" \
        HOME="${home_dir}" \
        BASH_ENV="$([[ "$verifiers" == "none" ]] && printf '%s' "${case_dir}/bash-env")" \
        /bin/bash "${INSTALLER}" --version v1.2.3 >"${log}" 2>&1
    local actual_status=$?
    set -e

    if [[ "$name" == "missing-verifier" ]]; then
        [[ "$actual_status" -ne 0 ]] || fail "${name}: expected failure"
    else
        [[ "$actual_status" -eq "$expected_status" ]] || fail "${name}: expected status ${expected_status}, got ${actual_status}"
    fi

    case "$name" in
        missing-verifier)
            /usr/bin/grep -Fqx 'Error: Neither sha256sum nor shasum found; cannot verify checksum.' "${log}" || {
                /bin/cat "${log}" >&2
                fail "${name}: missing deterministic diagnostic"
            }
            [[ ! -s "${case_dir}/events" ]] || fail "${name}: curl or install command ran"
            [[ ! -e "${home_dir}/.local/bin/oidc-exchange" ]] || fail "${name}: binary installed"
            ;;
        bad-checksum)
            [[ "$(<"${case_dir}/events")" == $'curl\ncurl\nsha256sum' ]] || fail "${name}: expected downloads followed by failed verification"
            [[ ! -e "${home_dir}/.local/bin/oidc-exchange" ]] || fail "${name}: binary installed"
            ;;
        sha256sum-precedence)
            [[ "$(<"${case_dir}/events")" == $'curl\ncurl\nsha256sum\nchmod\nmv' ]] || fail "${name}: sha256sum did not take precedence"
            ;;
        shasum)
            [[ "$(<"${case_dir}/events")" == $'curl\ncurl\nshasum\nchmod\nmv' ]] || fail "${name}: expected shasum verification followed by installation"
            ;;
    esac
}

run_case missing-verifier none 0 1
run_case bad-checksum sha256sum 1 1
run_case sha256sum-precedence both 0 0
run_case shasum shasum 0 0

printf 'install.sh verification gate tests passed\n'
