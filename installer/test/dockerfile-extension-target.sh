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
args="$(sed -nE 's/^ARG ((BEHAVIORAL_EXTENSION_|BUILD_FEATURES)[A-Z_]*)=(.*)$/\1=\3/p' "${DOCKERFILE}")"

# The whole RUN that fetches the extensions, not just its prologue: from its
# `RUN set -eu; \` through the last line of the continuation, which is the
# `cargo build` that ends it. (Other RUNs also open with `set -eu;`; the one
# that names the registry is the one wanted.)
whole="$(awk '
    /^RUN set -eu; \\$/ { grab = 1; buf = "" }
    grab {
        line = $0; cont = (line ~ /\\$/)
        sub(/[[:space:]]*\\$/, "", line); sub(/^RUN /, "", line)
        buf = buf line "\n"
        if (!cont) {
            grab = 0
            if (buf ~ /community-extensions\.duckdb\.org/) { print buf; exit }
        }
    }
' "${DOCKERFILE}")"

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

# run_whole <BUILD_FEATURES> <BEHAVIORAL_EXTENSION_OPTIONAL> <fetch: ok|fail>
# Runs the whole RUN with `curl`, `gunzip` and `cargo` stubbed, and prints what
# the stubbed `cargo` was handed. Exit status is the RUN's own.
run_whole() {
    (
        eval "${args}"
        # Exported, as a build ARG reaches a RUN: the body under test reads them by
        # name through the `eval` below, which shellcheck cannot follow (SC2034).
        export BUILD_FEATURES="$1"
        export BEHAVIORAL_EXTENSION_OPTIONAL="$2"
        export TARGETARCH=amd64
        FETCH="$3"
        curl() { [ "${FETCH}" = ok ]; }
        gunzip() { :; }
        cargo() { echo "cargo $*"; echo "behavioral=${BIRDNET_BUNDLED_EXTENSION_FILE:-unset} icu=${BIRDNET_BUNDLED_ICU_FILE:-unset}"; }
        eval "${whole}"
    ) 2>&1
}

echo "=== a failed fetch fails an image that embeds (the default features) ==="
out="$(run_whole analytics "" fail)"; rc=$?
if [ "${rc}" -ne 0 ] && grep -q 'ERROR: behavioral extension NOT fetched' <<<"${out}" && ! grep -q '^cargo ' <<<"${out}"; then
    pass "exit ${rc}, says so, and never reaches cargo build"
else
    fail "a failed fetch with the default features exited ${rc}: ${out}"
fi

echo "=== counterpart: the opt-out builds anyway, and says what the image lacks ==="
out="$(run_whole analytics 1 fail)"; rc=$?
if [ "${rc}" -eq 0 ] && grep -q 'WARNING: behavioral extension NOT fetched' <<<"${out}" && grep -q 'behavioral=unset icu=unset' <<<"${out}"; then
    pass "BEHAVIORAL_EXTENSION_OPTIONAL=1 -> exit 0, warning, nothing embedded"
else
    fail "the opt-out exited ${rc}: ${out}"
fi

echo "=== counterpart: a lean build embeds nothing, so a failed fetch is not an error ==="
out="$(run_whole "" "" fail)"; rc=$?
if [ "${rc}" -eq 0 ] && grep -q '^cargo build' <<<"${out}"; then
    pass "BUILD_FEATURES empty -> exit 0 and cargo build runs"
else
    fail "a lean build exited ${rc}: ${out}"
fi

echo "=== counterpart: when the fetch works, both files reach cargo build ==="
out="$(run_whole analytics "" ok)"; rc=$?
if [ "${rc}" -eq 0 ] && grep -q 'behavioral=/tmp/behavioral.duckdb_extension icu=/tmp/icu.duckdb_extension' <<<"${out}"; then
    pass "exit 0, BIRDNET_BUNDLED_EXTENSION_FILE and BIRDNET_BUNDLED_ICU_FILE set"
else
    fail "a working fetch exited ${rc}: ${out}"
fi

if [ "${FAILED}" -ne 0 ]; then
    echo "dockerfile-extension-target: FAILED"
    exit 1
fi
echo "dockerfile-extension-target: all pass"
