#!/usr/bin/env bash
# installer/test/model-pins-match-catalogue.sh — do the places that name a model
# agree about what it is, and does the shell stay out of BirdNET V2.4's way?
#
# A model is pinned by sha256 in places that nothing but a human keeps in step:
#
#   * crates/birdnet-integrations/src/model_catalog.rs — what
#     `birdnet-behavior --install-model` fetches, writes and verifies;
#   * installer/lib/10-config.sh (and the install.sh built from it) — what the
#     bare-metal installer fetches and verifies;
#   * docker/entrypoint.sh — what the container fetches and verifies.
#
# BirdNET V2.4 is not in the shell at all: the installer and the container run
# `birdnet-behavior --install-model birdnet-v24` and `... birdnet-v24-range-v2`,
# so the binary is the only place its digests, origins and sizes live, and the
# V2.4 labels (BirdNET-Pi's English names on V2.4's class order — a file the
# binary carries) are never downloaded. What the shell does hold are the NAMES
# the binary gives the files, so the config can point at them. This test holds:
#
#   1. those names equal what the catalogue's ids make `--install-model` write;
#   2. nothing of V2.4 is pinned or fetched in the shell: no digest of the model,
#      the range filter, or either labels file, and no URL for them. In
#      particular not birdnet-go's labels.txt, whose English names differ from
#      BirdNET-Pi's for 460 of 6 522 classes — a station that imported a
#      BirdNET-Pi history and then recorded with that file would hold one
#      species under two names;
#   3. the BirdNET+ V3.0 pair, which the shell still downloads, is pinned
#      identically in the installer, in install.sh, in the container and in the
#      catalogue (sha256, release tag, file names). Until this test nothing
#      checked that, and a digest mistyped in any one of them is an installer
#      that refuses a good model for ever;
#   4. the geomodel, which the catalogue does not carry, is pinned identically in
#      the installer and in the container.
#
# CATALOGUE=<path to model_catalog.rs> reads another revision of the catalogue
# (the check is expected to hold across the one that embeds the labels).
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
for must in "birdnet-v24:model_sha256" "birdnet-v24-range-v2:model_sha256" \
            "birdnet-v3:model_sha256" "birdnet-v3:labels_sha256"; do
    v="$(catalogue "${must%%:*}" "${must##*:}")"
    if [[ "${v}" =~ ^[0-9a-f]{64}$ ]]; then :; else
        fail "the catalogue parse found '${v}' for ${must}; the parser, not the pins, is broken"
    fi
done

SHELL_FILES=("${REPO_ROOT}/installer/lib/10-config.sh" "${REPO_ROOT}/install.sh" "${REPO_ROOT}/docker/entrypoint.sh")

# ── 1. the names ────────────────────────────────────────────────────────────
echo "=== 1. the file names are the ones --install-model gives ==="
# install() writes <id>.onnx for the model and for the range filter, and
# <id>_labels.<ext> for labels: ext is the labels URL's when the catalogue
# downloads them, and txt when the binary carries them (labels_url is None).
labels_url="$(catalogue birdnet-v24 labels_url)"
case "${labels_url}" in
    "" | NONE) ext="txt" ;;
    *) ext="${labels_url##*.}"; [ "${#ext}" -le 4 ] || ext="csv" ;;
esac
for f in "${SHELL_FILES[@]}"; do
    n="${f#"${REPO_ROOT}"/}"
    eq "${n}: V2.4 model id"         "$(shellval "${f}" V24_MODEL_ID)"    "birdnet-v24"
    eq "${n}: V2.4 range filter id"  "$(shellval "${f}" V24_RANGE_ID)"    "birdnet-v24-range-v2"
    eq "${n}: V2.4 model name"       "$(shellval "${f}" V24_MODEL_FILE)"  "birdnet-v24.onnx"
    eq "${n}: V2.4 labels name"      "$(shellval "${f}" V24_LABELS_FILE)" "birdnet-v24_labels.${ext}"
    eq "${n}: V2.4 range filter name" "$(shellval "${f}" V24_RANGE_FILE)" "birdnet-v24-range-v2.onnx"
done
# The ids the shell passes to the binary exist in the catalogue.
for id in birdnet-v24 birdnet-v24-range-v2; do
    if [ -n "$(catalogue "${id}" model_sha256)" ]; then pass "the catalogue has an entry '${id}'"; else fail "the catalogue has no entry '${id}'"; fi
done

# ── 2. nothing of V2.4 in the shell ─────────────────────────────────────────
echo "=== 2. no V2.4 digest or URL in anything the shell ships ==="
# The two labels digests are written here on purpose: this is the one place that
# says which file must NOT come from a shell download (the first) and which one
# the binary writes (the second). The model and range filter digests and URLs
# are read from the catalogue.
HF_LABELS_SHA="487937b6ad132b8506215523209a87b86adc9dce8e5ed3048ce9268189dddd3d"
EMBEDDED_LABELS_SHA="3773334c2ad218bde0ac083dd0b6e839a5cf34e84a6017830b9f446fbb57a215"
forbidden=(
    "$(catalogue birdnet-v24 model_sha256)"
    "$(catalogue birdnet-v24-range-v2 model_sha256)"
    "${HF_LABELS_SHA}"
    "${EMBEDDED_LABELS_SHA}"
    "huggingface.co/tphakala/BirdNET-v2.4"
    "BirdNET_MData_V2.onnx"
    "BirdNET_v2.4_fp32"
)
scan_files=("${REPO_ROOT}"/installer/lib/*.sh "${REPO_ROOT}/install.sh" "${REPO_ROOT}"/docker/*.sh "${REPO_ROOT}/quickstart.sh")
clean=1
for needle in "${forbidden[@]}"; do
    [ -n "${needle}" ] || continue
    hits="$(grep -lF -- "${needle}" "${scan_files[@]}" 2>/dev/null || true)"
    if [ -n "${hits}" ]; then
        clean=0
        fail "'${needle:0:48}…' appears in: $(echo "${hits}" | sed "s|${REPO_ROOT}/||" | tr '\n' ' ')"
    fi
done
[ "${clean}" = 1 ] && pass "none of the V2.4 digests, nor the Hugging Face and birdnet-go URLs, appear in installer/lib, install.sh, docker/ or quickstart.sh"

# ── 3. the V3.0 pair ────────────────────────────────────────────────────────
echo "=== 3. the V3.0 pair is pinned identically everywhere the shell downloads it ==="
V3_MODEL_CAT="$(catalogue birdnet-v3 model_sha256)"
V3_LABELS_CAT="$(catalogue birdnet-v3 labels_sha256)"
for f in "${SHELL_FILES[@]}"; do
    n="${f#"${REPO_ROOT}"/}"
    eq "${n}: V3.0 model sha256"  "$(shellval "${f}" V3_MODEL_SHA256)"  "${V3_MODEL_CAT}"
    eq "${n}: V3.0 labels sha256" "$(shellval "${f}" V3_LABELS_SHA256)" "${V3_LABELS_CAT}"
    # The catalogue spells the project's release URL; the shell pins the tag and
    # the file name separately. Re-assemble and compare.
    tag="$(shellval "${f}" V3_MODEL_RELEASE_TAG)"
    file="$(shellval "${f}" V3_MODEL_FILE)"
    labels_file="$(shellval "${f}" V3_LABELS_FILE)"
    base="https://github.com/tomtom215/BirdNet-Behavior/releases/download/${tag}"
    eq "${n}: V3.0 release tag and model name, as the catalogue's URL"  "${base}/${file//+/%2B}"        "$(catalogue birdnet-v3 model_url)"
    eq "${n}: V3.0 release tag and labels name, as the catalogue's URL" "${base}/${labels_file//+/%2B}" "$(catalogue birdnet-v3 labels_url)"
done

# ── 4. the geomodel ─────────────────────────────────────────────────────────
echo "=== 4. the installer and the container pin the geomodel identically ==="
# The geomodel is not in the catalogue (the Rust installer does not fetch it), so
# the only thing that can be held is that the two shell copies name the same
# bytes.
for k in GEOMODEL_VERSION GEOMODEL_FILE GEOMODEL_LABELS_FILE GEOMODEL_SHA256 GEOMODEL_LABELS_SHA256; do
    eq "${k}: installer and container" "$(shellval "${REPO_ROOT}/installer/lib/10-config.sh" "${k}")" "$(shellval "${REPO_ROOT}/docker/entrypoint.sh" "${k}")"
done
eq "V3_MODEL_RELEASE_TAG: installer and container" \
    "$(shellval "${REPO_ROOT}/installer/lib/10-config.sh" V3_MODEL_RELEASE_TAG)" "$(shellval "${REPO_ROOT}/docker/entrypoint.sh" V3_MODEL_RELEASE_TAG)"

echo
if [ "${FAILED}" -ne 0 ]; then
    echo "model-pins-match-catalogue.sh: FAILED"
    exit 1
fi
echo "model-pins-match-catalogue.sh: all pass"
