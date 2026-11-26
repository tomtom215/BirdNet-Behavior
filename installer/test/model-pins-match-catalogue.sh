#!/usr/bin/env bash
# installer/test/model-pins-match-catalogue.sh — do the places that name a model
# agree about what it is, and does the shell stay out of BirdNET V2.4's way?
#
# Every model a station runs is installed by the binary (MDL-09): install.sh,
# the macOS installer and the container's entrypoint all run
# `birdnet-behavior --install-model <id>`, and the catalogue
# (crates/birdnet-integrations/src/model_catalog.rs) is the only place a
# model's digests, sizes and origins live. What the shell does hold are the
# catalogue ids it asks for and the NAMES the binary gives the files, so a
# config can point at them — plus, for the BirdNET+ V3.0 files, the names this
# installer gave them before, so an existing station's config can be moved off
# them. This test holds:
#
#   1. every id the shell passes is a catalogue entry, and each filter is the
#      one its classifier pairs with;
#   2. every file name equals what that id makes `--install-model` write, and
#      every old V3.0 name equals the one the catalogue adopts;
#   3. no digest and no URL of any catalogue file — model, labels, any origin —
#      appears in anything the shell ships: no second copy to go stale, and no
#      download code to drift from the binary's. In particular not birdnet-go's
#      labels.txt, whose English names differ from BirdNET-Pi's for 460 of 6 522
#      classes.
#
# CATALOGUE=<path to model_catalog.rs> reads another revision of the catalogue.
#
# Usage: installer/test/model-pins-match-catalogue.sh
# Needs bash + coreutils + GNU sed. No network.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CATALOGUE="${CATALOGUE:-${REPO_ROOT}/crates/birdnet-integrations/src/model_catalog.rs}"
READ="${REPO_ROOT}/installer/catalogue-field.sh"
FAILED=0
pass() { printf '  PASS  %s\n' "$*"; }
fail() { printf '  FAIL  %s\n' "$*"; FAILED=1; }

[ -f "${CATALOGUE}" ] || { echo "no catalogue at ${CATALOGUE}"; exit 1; }
catalogue() { "${READ}" "$1" "$2" "${CATALOGUE}"; }

# shellval FILE NAME — NAME="value" or NAME=value at the start of a line.
shellval() {
    sed -n -e "s/^$2=\"\\(.*\\)\"\$/\\1/p" -e "s/^$2=\\([0-9][0-9]*\\)\$/\\1/p" "$1" | head -n 1
}

# eq WHAT GOT WANT
eq() {
    if [ -z "$2" ]; then
        fail "$1 — not found (wanted '$3')"
    elif [ "$2" = "$3" ]; then
        pass "$1"
    else
        fail "$1 — got '$2', the catalogue says '$3'"
    fi
}

# The catalogue must be readable, or every comparison below is vacuous.
IDS=(birdnet-v24 birdnet-v24-range-v2 birdnet-v3 birdnet-v3-geomodel)
for id in "${IDS[@]}"; do
    v="$(catalogue "${id}" model_sha256)"
    if [[ "${v}" =~ ^[0-9a-f]{64}$ ]]; then :; else
        fail "the catalogue parse found '${v}' for ${id}'s model_sha256; the parser, not the pins, is broken"
    fi
done

SHELL_FILES=("${REPO_ROOT}/installer/lib/10-config.sh" "${REPO_ROOT}/install.sh" "${REPO_ROOT}/docker/entrypoint.sh")

# ── 1. the ids ──────────────────────────────────────────────────────────────
echo "=== 1. the ids the shell asks for are catalogue entries, paired as the catalogue pairs them ==="
for f in "${SHELL_FILES[@]}"; do
    n="${f#"${REPO_ROOT}"/}"
    eq "${n}: V2.4 model id"         "$(shellval "${f}" V24_MODEL_ID)"   "birdnet-v24"
    eq "${n}: V2.4 range filter id"  "$(shellval "${f}" V24_RANGE_ID)"   "birdnet-v24-range-v2"
    eq "${n}: V3.0 model id"         "$(shellval "${f}" V3_MODEL_ID)"    "birdnet-v3"
    eq "${n}: V3.0 geomodel id"      "$(shellval "${f}" V3_GEOMODEL_ID)" "birdnet-v3-geomodel"
done
eq "the range filter pairs with V2.4" "$(catalogue birdnet-v24-range-v2 pairs_with)" "birdnet-v24"
eq "the geomodel pairs with V3.0"     "$(catalogue birdnet-v3-geomodel pairs_with)"   "birdnet-v3"

# ── 2. the names ────────────────────────────────────────────────────────────
echo "=== 2. the file names are the ones --install-model gives, and the old V3.0 names the ones it adopts ==="
# install() writes <id>.onnx for every model and <id>_labels.<labels_ext> for
# labels.
labels_name() { printf '%s_labels.%s' "$1" "$(catalogue "$1" labels_ext)"; }
for f in "${SHELL_FILES[@]}"; do
    n="${f#"${REPO_ROOT}"/}"
    eq "${n}: V2.4 model name"        "$(shellval "${f}" V24_MODEL_FILE)"       "birdnet-v24.onnx"
    eq "${n}: V2.4 labels name"       "$(shellval "${f}" V24_LABELS_FILE)"      "$(labels_name birdnet-v24)"
    eq "${n}: V2.4 range filter name" "$(shellval "${f}" V24_RANGE_FILE)"       "birdnet-v24-range-v2.onnx"
    eq "${n}: V3.0 model name"        "$(shellval "${f}" V3_MODEL_FILE)"        "birdnet-v3.onnx"
    eq "${n}: V3.0 labels name"       "$(shellval "${f}" V3_LABELS_FILE)"       "$(labels_name birdnet-v3)"
    eq "${n}: geomodel name"          "$(shellval "${f}" GEOMODEL_FILE)"        "birdnet-v3-geomodel.onnx"
    eq "${n}: geomodel labels name"   "$(shellval "${f}" GEOMODEL_LABELS_FILE)" "$(labels_name birdnet-v3-geomodel)"
    eq "${n}: old V3.0 model name"    "$(shellval "${f}" V3_LEGACY_MODEL_FILE)"        "$(catalogue birdnet-v3 legacy_model_name)"
    eq "${n}: old V3.0 labels name"   "$(shellval "${f}" V3_LEGACY_LABELS_FILE)"       "$(catalogue birdnet-v3 legacy_labels_name)"
    eq "${n}: old geomodel name"      "$(shellval "${f}" GEOMODEL_LEGACY_FILE)"        "$(catalogue birdnet-v3-geomodel legacy_model_name)"
    eq "${n}: old geomodel labels name" "$(shellval "${f}" GEOMODEL_LEGACY_LABELS_FILE)" "$(catalogue birdnet-v3-geomodel legacy_labels_name)"
done

# ── 3. nothing of any model's bytes or origins in the shell ─────────────────
echo "=== 3. no model digest or URL in anything the shell ships ==="
# birdnet-go's labels.txt digest is written here on purpose: it is the one file
# that must never come from a shell download.
HF_LABELS_SHA="487937b6ad132b8506215523209a87b86adc9dce8e5ed3048ce9268189dddd3d"
forbidden=("${HF_LABELS_SHA}" "zenodo.org" "huggingface.co" "birdnet-team/geomodel/releases"
           "BirdNET_MData_V2.onnx" "BirdNET_v2.4_fp32" "releases/download/models-")
for id in "${IDS[@]}" perch-v2; do
    for field in model_sha256 labels_sha256 model_url mirror_urls labels_url labels_mirror_urls; do
        v="$(catalogue "${id}" "${field}")"
        case "${v}" in "" | NONE) continue ;; esac
        forbidden+=("${v}")
    done
done
[ "${#forbidden[@]}" -ge 20 ] || fail "only ${#forbidden[@]} needles; the catalogue parse is short"
scan_files=("${REPO_ROOT}"/installer/lib/*.sh "${REPO_ROOT}/install.sh" "${REPO_ROOT}"/docker/*.sh "${REPO_ROOT}/quickstart.sh")
clean=1
for needle in "${forbidden[@]}"; do
    hits="$(grep -lF -- "${needle}" "${scan_files[@]}" 2>/dev/null || true)"
    if [ -n "${hits}" ]; then
        clean=0
        fail "'${needle:0:48}…' appears in: $(echo "${hits}" | sed "s|${REPO_ROOT}/||" | tr '\n' ' ')"
    fi
done
[ "${clean}" = 1 ] && pass "none of the ${#forbidden[@]} digests and URLs appear in installer/lib, install.sh, docker/ or quickstart.sh"

echo
if [ "${FAILED}" -ne 0 ]; then
    echo "model-pins-match-catalogue.sh: FAILED"
    exit 1
fi
echo "model-pins-match-catalogue.sh: all pass"
