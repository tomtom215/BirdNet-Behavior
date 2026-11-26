#!/usr/bin/env bash
# installer/test/macos-config.sh — what does the macOS install leave, and does it
# install the same classifier the Linux one does?
#
# The macOS path told the operator "(the BirdNET+ model downloads on first run)".
# Nothing downloads a model on first run: the binary has no downloader but
# `--install-model`, the installer wrote a config with no MODEL_PATH, and the
# Homebrew caveat said the same untrue thing about Zenodo. A Mac station
# detected nothing and said nothing about why. Now the macOS installer asks the
# binary for every model, as Linux does (MDL-09; it has no sha256sum to check
# files with, and the binary carries the pins), and writes a config that names
# the files:
#
#   M1  no options        -> BirdNET V2.4 and its range filter, live in the config
#   M2  birdnet-v3        -> the V3.0 preview and its geomodel, both live (MDL-17)
#   M3  a config naming a model of the operator's own -> left alone, the binary
#                            is not run
#   M3b a kept config of ours -> its models re-installed (retried, re-verified),
#                            and a filter that is now there switched on (MDL-14)
#   M4  a failed install  -> the config still names the files, the filter is not
#                            claimed, and the run ends as Linux's does: said
#                            loudly, last, and exit 75 (MDL-17)
#   M5  an unknown classifier is refused
#
# Run on Linux against the real functions (`macos_setup_config_and_agent` writes a
# config and a plist under $HOME and starts nothing), with the binary stubbed.
# What this cannot show: that any of it works on a Mac.
#
# Usage: installer/test/macos-config.sh
# Needs bash + coreutils. No root, no network.

set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=installer/test/lib/install-flow.sh
source "${HERE}/lib/install-flow.sh"

FAILED=0
pass() { printf '  PASS  %s\n' "$*"; }
fail() { printf '  FAIL  %s\n' "$*"; FAILED=1; }
SANDBOXES=()
trap 'rm -rf "${SANDBOXES[@]}"' EXIT

# run_macos <sandbox> — runs macos_setup_config_and_agent (REQUEST=… sets the
# classifier request; FLOW_FAIL_*=1 make the stub fail). Output in
# ${sb}/out.log; the calls the stub saw in ${sb}/requests.log.
run_macos() {
    local sb="$1"
    (
        export HOME="${sb}/home"
        export FLOW_REQUESTS="${sb}/requests.log"
        # shellcheck disable=SC1090
        source <(flow_script)
        mkdir -p "${sb}/bin"
        flow_write_stub_binary "${sb}/bin/birdnet-behavior"
        # shellcheck disable=SC2034
        CLASSIFIER_REQUEST="${REQUEST:-}"
        macos_setup_config_and_agent "${sb}/bin/birdnet-behavior"
    ) >"${sb}/out.log" 2>&1
}

new_sb() { SB="$(mktemp -d)"; SANDBOXES+=("${SB}"); mkdir -p "${SB}/home"; : >"${SB}/requests.log"; }
MAC() { printf '%s/home/Library/Application Support/birdnet-behavior' "$1"; }
conf() { grep -E "^$2=" "$(MAC "$1")/birdnet.conf" | tail -n 1 | cut -d= -f2-; }

echo "=== M1. no options: BirdNET V2.4 and its range filter ==="
new_sb
run_macos "${SB}"
rc=$?
[ "${rc}" -eq 0 ] && pass "the setup succeeded" || { fail "exit ${rc}"; sed 's/^/        /' "${SB}/out.log"; }
M="$(MAC "${SB}")/models"
[ "$(conf "${SB}" MODEL_PATH)" = "${M}/birdnet-v24.onnx" ] && pass "MODEL_PATH is the V2.4 model" || fail "MODEL_PATH is '$(conf "${SB}" MODEL_PATH)'"
[ "$(conf "${SB}" LABELS_PATH)" = "${M}/birdnet-v24_labels.txt" ] && pass "LABELS_PATH is the V2.4 labels" || fail "LABELS_PATH is '$(conf "${SB}" LABELS_PATH)'"
[ "$(conf "${SB}" METADATA_MODEL_PATH)" = "${M}/birdnet-v24-range-v2.onnx" ] && pass "METADATA_MODEL_PATH is the range filter, live" || fail "METADATA_MODEL_PATH is '$(conf "${SB}" METADATA_MODEL_PATH)'"
if grep -qx "INSTALL-MODEL birdnet-v24 dir=${M}" "${SB}/requests.log" && grep -qx "INSTALL-MODEL birdnet-v24-range-v2 dir=${M}" "${SB}/requests.log" \
    && [ -s "${M}/birdnet-v24.onnx" ] && [ -s "${M}/birdnet-v24-range-v2.onnx" ]; then
    pass "the binary was asked for both, into the models directory, and wrote them"
else
    fail "binary calls: $(tr '\n' ';' <"${SB}/requests.log")"
fi
! grep -q 'CHILD-SAW' "${SB}/requests.log" && pass "the binary is run without the installer's own BIRDNET_* settings" || fail "the binary saw: $(grep CHILD-SAW "${SB}/requests.log" | tr '\n' ' ')"
if grep -q 'downloads on first run' "${SB}/out.log"; then fail "the output still says a model 'downloads on first run'"; else pass "and nothing claims the model downloads by itself"; fi

echo "=== M2. counterpart: birdnet-v3 is the V3.0 preview, with its geomodel ==="
new_sb
REQUEST=birdnet-v3 run_macos "${SB}"
M="$(MAC "${SB}")/models"
[ "$(conf "${SB}" MODEL_PATH)" = "${M}/birdnet-v3.onnx" ] && [ "$(conf "${SB}" LABELS_PATH)" = "${M}/birdnet-v3_labels.csv" ] \
    && pass "MODEL_PATH and LABELS_PATH are the V3.0 files" || fail "MODEL_PATH='$(conf "${SB}" MODEL_PATH)' LABELS_PATH='$(conf "${SB}" LABELS_PATH)'"
[ "$(conf "${SB}" METADATA_MODEL_PATH)" = "${M}/birdnet-v3-geomodel.onnx" ] && [ "$(conf "${SB}" METADATA_LABELS_PATH)" = "${M}/birdnet-v3-geomodel_labels.txt" ] \
    && pass "the geomodel and its labels are live: a V3.0 Mac has its occurrence filter" \
    || fail "METADATA_MODEL_PATH='$(conf "${SB}" METADATA_MODEL_PATH)' METADATA_LABELS_PATH='$(conf "${SB}" METADATA_LABELS_PATH)'"
if [ "$(grep -c '^INSTALL-MODEL' "${SB}/requests.log")" = 2 ] && grep -qx "INSTALL-MODEL birdnet-v3 dir=${M}" "${SB}/requests.log" \
    && grep -qx "INSTALL-MODEL birdnet-v3-geomodel dir=${M}" "${SB}/requests.log"; then
    pass "the binary was asked for birdnet-v3 and birdnet-v3-geomodel"
else
    fail "binary calls: $(tr '\n' ';' <"${SB}/requests.log")"
fi

echo "=== M3. an existing config naming a model of the operator's own is left alone ==="
new_sb
mkdir -p "$(MAC "${SB}")"
printf 'SITENAME=Mine\nMODEL_PATH=/somewhere/mine.onnx\n' >"$(MAC "${SB}")/birdnet.conf"
before="$(cat "$(MAC "${SB}")/birdnet.conf")"
REQUEST=birdnet-v24 run_macos "${SB}"
[ "$(cat "$(MAC "${SB}")/birdnet.conf")" = "${before}" ] && pass "the config is byte-identical" || fail "the config changed"
[ ! -s "${SB}/requests.log" ] && pass "and the binary was not asked to install anything" || fail "the binary was run: $(tr '\n' ';' <"${SB}/requests.log")"

echo "=== M3b. a kept config of ours: its models re-checked, and a filter now there switched on ==="
new_sb
FLOW_FAIL_RANGE=1 run_macos "${SB}"
M="$(MAC "${SB}")/models"
grep -qE "^# METADATA_MODEL_PATH=${M}/birdnet-v24-range-v2.onnx\$" "$(MAC "${SB}")/birdnet.conf" \
    && pass "precondition: the first run left the filter commented out" || fail "precondition: $(grep METADATA "$(MAC "${SB}")/birdnet.conf")"
: >"${SB}/requests.log"
run_macos "${SB}"
if grep -qx "INSTALL-MODEL birdnet-v24 dir=${M}" "${SB}/requests.log" && grep -qx "INSTALL-MODEL birdnet-v24-range-v2 dir=${M}" "${SB}/requests.log"; then
    pass "the re-run asked the binary for the classifier and its filter again"
else
    fail "binary calls: $(tr '\n' ';' <"${SB}/requests.log")"
fi
[ "$(conf "${SB}" METADATA_MODEL_PATH)" = "${M}/birdnet-v24-range-v2.onnx" ] && [ "$(conf "${SB}" MODEL_PATH)" = "${M}/birdnet-v24.onnx" ] \
    && pass "the filter is live now that it is there; MODEL_PATH is as it was" || fail "METADATA_MODEL_PATH is '$(conf "${SB}" METADATA_MODEL_PATH)'"

# A V3.0 config the previous macOS installer wrote: the geomodel's placeholder,
# and no labels line. The re-run installs the geomodel and names both files.
new_sb
mkdir -p "$(MAC "${SB}")"
M="$(MAC "${SB}")/models"
printf 'SITENAME=Mine\nMODEL_PATH=%s/birdnet-v3.onnx\nLABELS_PATH=%s/birdnet-v3_labels.csv\n# METADATA_MODEL_PATH=\n' "${M}" "${M}" \
    >"$(MAC "${SB}")/birdnet.conf"
run_macos "${SB}"
[ "$(conf "${SB}" METADATA_MODEL_PATH)" = "${M}/birdnet-v3-geomodel.onnx" ] && [ "$(conf "${SB}" METADATA_LABELS_PATH)" = "${M}/birdnet-v3-geomodel_labels.txt" ] \
    && [ "$(conf "${SB}" SITENAME)" = "Mine" ] \
    && pass "an old macOS V3.0 config gets the geomodel and its labels, and keeps the rest" \
    || fail "old macOS V3.0 config: $(tr '\n' '|' <"$(MAC "${SB}")/birdnet.conf")"

echo "=== M4. an install that fails: the config names the files, and the run ends loudly with 75 ==="
new_sb
FLOW_FAIL_MODEL=1 FLOW_FAIL_RANGE=1 run_macos "${SB}"
rc=$?
M="$(MAC "${SB}")/models"
[ "${rc}" -eq 0 ] && pass "the setup carried on" || fail "exit ${rc}"
[ "$(conf "${SB}" MODEL_PATH)" = "${M}/birdnet-v24.onnx" ] && pass "MODEL_PATH still names the V2.4 model, ready for a retry" || fail "MODEL_PATH is '$(conf "${SB}" MODEL_PATH)'"
[ -z "$(conf "${SB}" METADATA_MODEL_PATH)" ] && pass "the range filter is not claimed" || fail "a range filter that is not on disk is live"
grep -q 'detects nothing until they are installed' "${SB}/out.log" \
    && pass "the output says nothing detects until the files are installed" || fail "the failure was not explained"
# The whole macOS run, through main, as the operator starts it.
new_sb
(
    export HOME="${SB}/home"
    export FLOW_REQUESTS="${SB}/requests.log"
    export BIRDNET_NONINTERACTIVE=1
    # shellcheck disable=SC1090
    source <(flow_script)
    mkdir -p "${SB}/bin"
    flow_write_stub_binary "${SB}/bin/birdnet-behavior"
    uname() { [ "${1:-}" = "-s" ] && echo Darwin || command uname "$@"; }
    macos_install() { macos_setup_config_and_agent "${SB}/bin/birdnet-behavior"; }
    FLOW_FAIL_MODEL=1 main
) >"${SB}/out.log" 2>&1
rc=$?
[ "${rc}" -eq 75 ] && pass "the macOS run exits 75 (installed, not detecting), as Linux's does" || fail "the macOS run exited ${rc}, not 75"
tail_text="$(tail -n 14 "${SB}/out.log")"
if grep -q 'NOT DETECTING BIRDS' <<<"${tail_text}" && grep -q -- '--local-model' <<<"${tail_text}" && grep -q 'status 75' <<<"${tail_text}"; then
    pass "and it ends saying the station is not detecting, how to install the model, and the status"
else
    fail "the end of the run: $(tail -8 "${SB}/out.log" | tr '\n' '|')"
fi
new_sb
(
    export HOME="${SB}/home"
    export FLOW_REQUESTS="${SB}/requests.log"
    export BIRDNET_NONINTERACTIVE=1
    # shellcheck disable=SC1090
    source <(flow_script)
    mkdir -p "${SB}/bin"
    flow_write_stub_binary "${SB}/bin/birdnet-behavior"
    uname() { [ "${1:-}" = "-s" ] && echo Darwin || command uname "$@"; }
    macos_install() { macos_setup_config_and_agent "${SB}/bin/birdnet-behavior"; }
    main
) >"${SB}/out.log" 2>&1
rc=$?
[ "${rc}" -eq 0 ] && ! grep -q 'NOT DETECTING' "${SB}/out.log" \
    && pass "counterpart: a macOS run whose models install exits 0, with no such warning" || fail "a good macOS run exited ${rc}"

echo "=== M5. an unknown classifier is refused ==="
new_sb
REQUEST=birdnet-v4 run_macos "${SB}"
rc=$?
if [ "${rc}" -ne 0 ] && [ ! -e "$(MAC "${SB}")/birdnet.conf" ] && [ ! -s "${SB}/requests.log" ]; then
    pass "refused, nothing written, the binary not run"
else
    fail "exit ${rc}; config $([ -e "$(MAC "${SB}")/birdnet.conf" ] && echo written || echo absent)"
fi

echo
if [ "${FAILED}" -ne 0 ]; then
    echo "macos-config.sh: FAILED"
    exit 1
fi
echo "macos-config.sh: all pass"
