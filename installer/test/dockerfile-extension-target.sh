#!/usr/bin/env bash
# installer/test/dockerfile-extension-target.sh — does a local image build
# embed the DuckDB extensions for the platform it is built for?
#
# The Dockerfile's builder stage downloads the `behavioral` and `icu` DuckDB
# extensions and embeds them, so the station loads them with no network. The
# platform in their URL came from `ARG BEHAVIORAL_EXTENSION_TARGET` defaulting
# to `linux_amd64`. docker.yml passes the right value per arch, but a plain
# `docker compose build` on a Raspberry Pi did not — it embedded amd64 builds
# the arm64 engine cannot LOAD, and the "offline" image needed the network at
# first run after all.
#
# This evaluates the Dockerfile's own RUN prologue — ARG defaults applied the
# way BuildKit applies them, up to the line that builds the extension URL — and
# checks the URL it would fetch for each platform.
#
# Usage: installer/test/dockerfile-extension-target.sh
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DOCKERFILE="${REPO_ROOT}/Dockerfile"
FAILED=0
pass() { printf '  PASS  %s\n' "$*"; }
fail() { printf '  FAIL  %s\n' "$*"; FAILED=1; }

# The shell of the RUN that fetches the extension, from `RUN set -eu; \` up to
# and including the `url=` line, continuation backslashes removed.
prologue="$(awk '
    /^RUN set -eu; \\$/ { grab = 1; buf = "" }
    grab { line = $0; sub(/[[:space:]]*\\$/, "", line); sub(/^RUN /, "", line); buf = buf line "\n" }
    grab && /url="https:\/\/community-extensions\.duckdb\.org\// { print buf; exit }
' "${DOCKERFILE}")"
if [ -z "${prologue}" ]; then
    fail "could not find the extension-fetching RUN in the Dockerfile"
    exit 1
fi

# The ARG defaults that RUN sees, as shell assignments.
args="$(sed -nE 's/^ARG (BEHAVIORAL_EXTENSION_[A-Z_]+)=(.*)$/\1=\2/p' "${DOCKERFILE}")"

# url_for <TARGETARCH as BuildKit sets it> [explicit --build-arg target]
url_for() {
    (
        eval "${args}"
        # shellcheck disable=SC2034  # read by the evaluated prologue
        TARGETARCH="$1"
        # shellcheck disable=SC2034  # read by the evaluated prologue
        if [ -n "${2:-}" ]; then BEHAVIORAL_EXTENSION_TARGET="$2"; fi
        eval "${prologue}" >/dev/null
        # shellcheck disable=SC2154  # set by the evaluated prologue
        printf '%s\n' "${url}"
    )
}

echo "=== a build for arm64 embeds the arm64 extension ==="
got="$(url_for arm64)"
case "${got}" in
    */linux_arm64/behavioral.duckdb_extension.gz) pass "arm64 → $(basename "$(dirname "${got}")")" ;;
    *) fail "an arm64 build fetches ${got}" ;;
esac

echo "=== counterpart: a build for amd64 still embeds the amd64 extension ==="
got="$(url_for amd64)"
case "${got}" in
    */linux_amd64/behavioral.duckdb_extension.gz) pass "amd64 → $(basename "$(dirname "${got}")")" ;;
    *) fail "an amd64 build fetches ${got}" ;;
esac

echo "=== without BuildKit (no TARGETARCH) the stage's own architecture is used ==="
case "$(uname -m)" in x86_64) want=linux_amd64 ;; aarch64|arm64) want=linux_arm64 ;; *) want="linux_$(uname -m)" ;; esac
got="$(url_for "")"
case "${got}" in
    */"${want}"/behavioral.duckdb_extension.gz) pass "no TARGETARCH on $(uname -m) → ${want}" ;;
    *) fail "with no TARGETARCH on $(uname -m) it fetches ${got}" ;;
esac

echo "=== counterpart: an explicit --build-arg (what docker.yml passes) wins ==="
got="$(url_for amd64 linux_arm64)"
case "${got}" in
    */linux_arm64/behavioral.duckdb_extension.gz) pass "explicit linux_arm64 is used" ;;
    *) fail "the explicit build-arg was ignored: ${got}" ;;
esac

if [ "${FAILED}" -ne 0 ]; then
    echo "dockerfile-extension-target: FAILED"
    exit 1
fi
echo "dockerfile-extension-target: all pass"
