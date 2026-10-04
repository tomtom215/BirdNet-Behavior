#!/usr/bin/env bash
# installer/test/rollback-geomodel.sh — does the rollback an update prints
# give a pre-geomodel binary back the config it ran on?
#
# An update that finds no METADATA_MODEL_PATH in the kept config adds the
# geomodel to it. Such a config was written before the geomodel shipped, so
# the binary kept as .prev predates it too: rolled back under the plain
# `mv .prev`, 0.15.0 loaded the 12K-output geomodel, paired its outputs with
# the 11K classifier by position, and with coordinates set recorded nothing
# from a magpie recording. The update now prints a rollback that also removes
# the lines it added. Run that printed command and check what it leaves.
#
# Usage: installer/test/rollback-geomodel.sh
# Needs bash + coreutils + GNU sed. No root.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
FAILED=0
pass() { printf '  PASS  %s\n' "$*"; }
fail() { printf '  FAIL  %s\n' "$*"; FAILED=1; }

WORK="$(mktemp -d)"
trap 'rm -rf "${WORK}"' EXIT

ORIGINAL='# station config
LATITUDE=52.5200
LONGITUDE=13.4050
ALSA_CARD=plughw:1,0'

# Run enable_geomodel_in_kept_config on $1 (the config's starting text) and
# print what it said.
run_update() {
    mkdir -p "${WORK}/bin" "${WORK}/stub"
    printf '%s\n' "$1" > "${WORK}/birdnet.conf"
    printf 'new\n' > "${WORK}/bin/birdnet-behavior"
    printf 'old\n' > "${WORK}/bin/birdnet-behavior.prev"
    printf '#!/bin/sh\nexit 0\n' > "${WORK}/stub/systemctl"
    chmod +x "${WORK}/stub/systemctl"
    (
        set -euo pipefail
        # shellcheck disable=SC1090
        source <(sed -n '/^enable_geomodel_in_kept_config()/,/^}/p' "${REPO_ROOT}/installer/lib/62-config-file.sh")
        info()    { printf '%s\n' "$*"; }
        success() { printf '%s\n' "$*"; }
        warn()    { printf '%s\n' "$*"; }
        # shellcheck disable=SC2034  # all read by the sourced function
        {
            GEOMODEL_INSTALLED=1
            MODEL_DIR="/home/pi/BirdNet-Behavior/models"
            GEOMODEL_FILE="BirdNET+_Geomodel_V3.0.2_Global_12K_FP32.onnx"
            GEOMODEL_LABELS_FILE="BirdNET+_Geomodel_V3.0.2_Global_12K_Labels.txt"
            CONFIG_FILE="${WORK}/birdnet.conf"
            INSTALL_DIR="${WORK}/bin"
            BINARY_NAME="birdnet-behavior"
            SERVICE_NAME="birdnet-behavior.service"
        }
        enable_geomodel_in_kept_config
    )
}

# The last rollback command printed, run as the operator would (we are not
# root, and need not be: every path is ours).
run_rollback() {
    local cmd
    cmd="$(grep -o 'roll back with: .*' <<<"$1" | tail -1 | sed 's/^roll back with: //')"
    [ -n "${cmd}" ] || return 1
    PATH="${WORK}/stub:${PATH}" bash -c "${cmd//sudo /}"
}

echo "=== 1. a pre-geomodel config gets a rollback that takes the geomodel back out ==="
out="$(run_update "${ORIGINAL}")"
if grep -q '^METADATA_MODEL_PATH=' "${WORK}/birdnet.conf"; then
    pass "the update added the geomodel (precondition)"
else
    fail "the update added no geomodel, so nothing below is tested"
fi
# The operator edits the config after updating; that edit is theirs to keep.
printf 'RTSP_URL=rtsp://cam\n' >> "${WORK}/birdnet.conf"
if run_rollback "${out}"; then
    pass "the printed rollback ran"
else
    fail "no rollback command was printed, or it failed: ${out}"
fi
if grep -q 'METADATA_' "${WORK}/birdnet.conf"; then
    fail "the rolled-back config still names the geomodel: $(grep METADATA_ "${WORK}/birdnet.conf" | tr '\n' ' ')"
else
    pass "the rolled-back config no longer names the geomodel"
fi
if diff -B <(printf '%s\nRTSP_URL=rtsp://cam\n' "${ORIGINAL}") "${WORK}/birdnet.conf" >/dev/null; then
    pass "everything else is as it was, the operator's later edit included"
else
    fail "the rollback changed more than the geomodel: $(diff -B <(printf '%s\nRTSP_URL=rtsp://cam\n' "${ORIGINAL}") "${WORK}/birdnet.conf" | tr '\n' ' ')"
fi
if [ "$(cat "${WORK}/bin/birdnet-behavior")" = old ] && [ ! -e "${WORK}/bin/birdnet-behavior.prev" ]; then
    pass "the previous binary is back in place"
else
    fail "the previous binary was not moved back"
fi

echo
echo "=== 2. counterpart: a config that already names a geomodel keeps it ==="
out="$(run_update "${ORIGINAL}
METADATA_MODEL_PATH=/srv/my-own-geomodel.onnx")"
if grep -q 'predates the geomodel' <<<"${out}"; then
    fail "an operator's own geomodel setting was offered for removal: ${out}"
else
    pass "no geomodel-removing rollback when the config already had one"
fi

echo
if [ "${FAILED}" -eq 0 ]; then
    echo "rollback-geomodel: all pass"
else
    echo "rollback-geomodel: FAILURES"
fi
exit "${FAILED}"
