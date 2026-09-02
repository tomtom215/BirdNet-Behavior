#!/usr/bin/env bash
# installer/test/config-template.sh — does write_config emit the config file it
# means to, or does the shell get to it first?
#
# The template in installer/lib/62-config-file.sh is one ~100-line unquoted
# heredoc (`cat > "${CONFIG_FILE}" <<EOF`). Unquoted is deliberate: the whole
# point is that `${MODEL_DIR}`, `${LISTEN_ADDR}` and the rest interpolate. The
# cost is that everything *else* the shell finds special interpolates too, and
# in a comment that is invisible on review — a backticked command in prose
# reads as documentation and executes as a subshell.
#
# That is not hypothetical. A "# Run `birdnet-behavior --doctor` …" hint added
# to the species-filter section shipped inside this heredoc and would have run
# the diagnostic during install, pasting its multi-line report into the middle
# of the operator's config file. The file had contained no backticks at all
# before that, so nothing was watching for the first one.
#
# So this test does two things:
#   1. Checks the behaviour that section is for: the METADATA_* settings are
#      live when the geomodel was installed and commented out when it was not.
#   2. Runs write_config with every command the template mentions replaced by a
#      sentinel that screams, and greps the result. A comment that executes is
#      caught by what it produced, not by a rule about how to write comments.
#
# Usage: installer/test/config-template.sh
# Needs bash + coreutils. No root, no network.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
FAILED=0

pass() { printf '  PASS  %s\n' "$*"; }
fail() { printf '  FAIL  %s\n' "$*"; FAILED=1; }

WORK="$(mktemp -d)"
trap 'rm -rf "${WORK}"' EXIT

# Render the config with write_config from the real module.
#
#   $1 — GEOMODEL_INSTALLED value ("1" or "0")
#   $2 — destination path
#
# Every helper the function calls is stubbed, including chown/chmod, which need
# root and are not what this test is about. `birdnet-behavior` and `date` are
# stubbed to sentinels: if the heredoc ever executes prose again, the sentinel
# lands in the file and the grep below sees it.
# `write_config` is pulled in below by `source <(sed ...)`, a process
# substitution the linter cannot follow even with `-x`. Every variable this
# function assigns is read by it (installer/lib/62-config-file.sh), so they all
# look assigned-and-never-used from here. The directive is scoped to this
# function and to SC2034 alone, so a real finding anywhere else in this file
# still fails the build.
#
# (No line of this comment may begin with the linter's own name: it parses such
# a line as a directive and errors on the prose.)
# shellcheck disable=SC2034
render() {
    local geo_installed="$1" dest="$2"
    (
        set -uo pipefail
        # shellcheck disable=SC1090
        source <(sed -n '/^write_config()/,/^}/p' "${REPO_ROOT}/installer/lib/62-config-file.sh")

        info()    { :; }
        success() { :; }
        warn()    { :; }
        chown()   { :; }
        chmod()   { :; }
        # Any command named in the template must not run. If one does, its
        # output is unmistakable in the rendered file.
        birdnet-behavior() { echo "SENTINEL_EXECUTED_birdnet_behavior"; }
        systemctl()        { echo "SENTINEL_EXECUTED_systemctl"; }

        CONFIG_FILE="${dest}"
        GEOMODEL_INSTALLED="${geo_installed}"

        MODEL_DIR="/var/lib/birdnet/models"
        MODEL_FILE="Classifier.onnx"
        LABELS_FILE="Classifier_Labels.csv"
        GEOMODEL_FILE="Geomodel.onnx"
        GEOMODEL_LABELS_FILE="Geomodel_Labels.txt"
        DATA_DIR="/var/lib/birdnet"
        IMAGE_CACHE_DIR="/var/lib/birdnet/image_cache"
        STREAM_DIR="/tmp/birdnet-stream"
        RECS_DIR="/var/lib/birdnet/BirdSongs"
        DB_PATH="/var/lib/birdnet/birds.db"
        ANALYTICS_DB_PATH="/var/lib/birdnet/analytics.duckdb"
        SERVICE_USER="birdnet"
        LISTEN_ADDR="0.0.0.0:8502"
        ALSA_CARD_VALUE=""
        RTSP_URL_VALUE=""
        LATITUDE_VALUE=""
        LONGITUDE_VALUE=""
        CADDY_USER_VALUE=""
        CADDY_PWD_VALUE=""

        write_config
    ) >/dev/null 2>&1
}

echo "=== 1. the template renders as text, not as commands ==="

ON="${WORK}/on.conf"
render 1 "${ON}"

if [ ! -s "${ON}" ]; then
    fail "write_config produced no file"
    echo "config-template: FAILURES"
    exit 1
fi
pass "write_config wrote a config"

if grep -q "SENTINEL_EXECUTED" "${ON}"; then
    fail "the heredoc executed a command that should have stayed prose:"
    grep -n "SENTINEL_EXECUTED" "${ON}" | sed 's/^/        /'
    printf '        %s\n' \
        "The template is an unquoted heredoc, so backticks and \$(...) run." \
        "Escape them (\\\` / \\\$) or the operator's config gets command output."
else
    pass "no command in the template executed"
fi

# The hint itself must survive verbatim — an escape that renders as a literal
# backslash would be just as wrong as one that executes.
if grep -qF -- '# Run `birdnet-behavior --doctor` to see which of the three is missing.' "${ON}"; then
    pass "the --doctor hint is written verbatim, backticks intact"
else
    fail "the --doctor hint is missing or mangled; the rendered line was:"
    grep -n -- "--doctor" "${ON}" | sed 's/^/        /' || printf '        (no line mentions --doctor)\n'
fi

echo "=== 2. the geomodel settings follow what was actually installed ==="

if grep -qx "METADATA_MODEL_PATH=/var/lib/birdnet/models/Geomodel.onnx" "${ON}" &&
    grep -qx "METADATA_LABELS_PATH=/var/lib/birdnet/models/Geomodel_Labels.txt" "${ON}"; then
    pass "GEOMODEL_INSTALLED=1 writes both settings live, with the real paths"
else
    fail "GEOMODEL_INSTALLED=1 did not write live METADATA_* settings; got:"
    grep -n "METADATA_" "${ON}" | sed 's/^/        /'
fi

OFF="${WORK}/off.conf"
render 0 "${OFF}"

# The counterpart. Without it, a template that hardcoded the paths would pass
# the check above while pointing every station that declined the download at
# files it does not have — which --doctor would then report as FAIL.
if grep -qE "^# METADATA_MODEL_PATH=" "${OFF}" &&
    grep -qE "^# METADATA_LABELS_PATH=" "${OFF}" &&
    ! grep -qE "^METADATA_(MODEL|LABELS)_PATH=" "${OFF}"; then
    pass "GEOMODEL_INSTALLED=0 leaves both settings commented out"
else
    fail "GEOMODEL_INSTALLED=0 must not point the station at absent files; got:"
    grep -n "METADATA_" "${OFF}" | sed 's/^/        /'
fi

if [ "${FAILED}" -eq 0 ]; then
    echo "config-template: OK"
else
    echo "config-template: FAILURES"
fi
exit "${FAILED}"
