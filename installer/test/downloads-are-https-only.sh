#!/usr/bin/env bash
# installer/test/downloads-are-https-only.sh — can a redirect move an installer
# download onto plain http?
#
# The installer reads SHA256SUMS over the same connection as the archive it
# checks, so the connection is the trust anchor: `curl -fsSL` with no
# `--proto` follows a redirect to http:// without a word, and `wget` does the
# same. Every origin the installer talks to is https (GitHub and the model
# hosts), so each download helper now says so.
#
# Each helper is run as it ships with `curl` and `wget` replaced by stubs that
# record their arguments; the question is what the helper asked of them.
#
# Usage: installer/test/downloads-are-https-only.sh
#        DOWNLOAD_LIB=<path> installer/test/downloads-are-https-only.sh   (to
#        point it at another copy of 40-download.sh)
# Needs only bash + coreutils. Exit 0 = all pass.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
LIB="${DOWNLOAD_LIB:-${REPO_ROOT}/installer/lib/40-download.sh}"
FAILED=0
pass() { printf '  PASS  %s\n' "$*"; }
fail() { printf '  FAIL  %s\n' "$*"; FAILED=1; }

SANDBOX="$(mktemp -d)"
trap 'rm -rf "${SANDBOX}"' EXIT

# A stub that records its arguments, writes the file it was asked for, and
# prints an HTTP status when curl is asked for one (-w '%{http_code}').
make_stub() { # <dir> <name>
    mkdir -p "$1"
    cat >"$1/$2" <<'STUB'
#!/bin/bash
printf '%s\n' "$*" >>"${STUB_LOG}"
out=""
prev=""
for a in "$@"; do
    if [ "${prev}" = "-o" ] || [ "${prev}" = "-O" ]; then out="${a}"; fi
    prev="${a}"
done
[ -n "${out}" ] && : >"${out}"
case "$*" in *http_code*) printf '200' ;; esac
exit 0
STUB
    chmod +x "$1/$2"
}

run_helper() { # <path-dir> <helper> [no-curl] -> echoes the recorded argument line(s)
    local pathdir="$1" helper="$2" nocurl="${3:-}"
    export STUB_LOG="${SANDBOX}/log.$$.${RANDOM}"
    : >"${STUB_LOG}"
    (
        PATH="${pathdir}:${PATH}"
        if [ -n "${nocurl}" ]; then
            # Make the helper believe curl is absent, so it takes the wget branch.
            command() {
                if [ "$1" = "-v" ] && [ "$2" = "curl" ]; then return 1; fi
                builtin command "$@"
            }
        fi
        # shellcheck disable=SC1090
        source "${LIB}"
        info() { :; }
        fatal() { echo "fatal: $*" >&2; exit 1; }
        "${helper}" "https://example.invalid/a" "${SANDBOX}/dest.$$"
    ) >/dev/null 2>&1
    cat "${STUB_LOG}"
}

make_stub "${SANDBOX}/with-curl" curl
make_stub "${SANDBOX}/with-wget" wget

for helper in download download_or_absent download_large; do
    args="$(run_helper "${SANDBOX}/with-curl" "${helper}")"
    if [[ "${args}" == *"--proto =https"* && "${args}" == *"--proto-redir =https"* && "${args}" == *"--tlsv1.2"* ]]; then
        pass "${helper} (curl) refuses a non-https origin or redirect and TLS older than 1.2"
    else
        fail "${helper} (curl) did not restrict the protocol; it ran: ${args:-<nothing>}"
    fi
done

for helper in download download_large; do
    args="$(run_helper "${SANDBOX}/with-wget" "${helper}" no-curl)"
    if [[ "${args}" == *"--https-only"* ]]; then
        pass "${helper} (wget) is https-only"
    else
        fail "${helper} (wget) did not pass --https-only; it ran: ${args:-<nothing>}"
    fi
done

exit "${FAILED}"
