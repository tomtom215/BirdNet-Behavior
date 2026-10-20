#!/usr/bin/env bash
# installer/test/classifier-default.sh — what does a NEW install, with nothing
# customised, put on the station?
#
# The maintainer's decision: a user who installs with no options must get
# detections they can trust the way BirdNET-Pi users have for years. BirdNET-Pi
# and birdnet-go run BirdNET V2.4; the installer used to default to BirdNET+
# V3.0 preview, a different network, so out of the box the three projects
# disagreed. The default is now V2.4 with its range filter, and V3.0 is an
# explicit `--classifier birdnet-v3`.
#
# V2.4 is installed by the binary (`birdnet-behavior --install-model`), which
# owns its digests, its origins and the labels file it carries. So this drives
# the real `main` (installer/test/lib/install-flow.sh) with a stub binary, and
# reads what the installer did with it: the config, the models directory, the
# ids it asked the binary to install, the URLs it fetched itself (V3.0 only),
# and what it told the operator.
#
#   1. no options            -> V2.4: the binary is asked for the model and for
#                               the range filter, both live in the config;
#                               BirdNET-Pi's defaults documented but commented;
#                               nothing of V3.0 fetched, nothing of V2.4 fetched
#                               by the installer itself.
#   2. --classifier birdnet-v3 -> the previous behaviour, file for file.
#   3. the same by environment, and the flag beating it.
#   4. a typo is refused before anything is downloaded.
#   5. the range filter's failure does not fail the install, and is not hidden.
#   6. the classifier's failure does, and says what to do.
#   7. BIRDNET_SKIP_MODEL=1 stages nothing and says which files to stage.
#   8. the free-space floor follows the classifier.
#   9. --help documents the flag.
#
# Counterparts are marked: (1) and (2) pin opposite ends of the same switch, so
# a default that ignores the flag, or a flag that ignores the default, fails one
# of them.
#
# Usage: installer/test/classifier-default.sh
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
new_sandbox() { SB="$(flow_new)"; SANDBOXES+=("${SB}"); }

# The paths a station's config is expected to carry in sandbox $1.
want() { printf '%s/%s' "$(flow_models_dir "$1")" "$2"; }

# ── 1. no options ───────────────────────────────────────────────────────────
echo "=== 1. a new install with no options runs BirdNET V2.4 and its range filter ==="
new_sandbox
flow_run "${SB}"
rc=$?
CFG="$(flow_config "${SB}")"
[ "${rc}" -eq 0 ] && pass "the install succeeded" || { fail "the install failed (exit ${rc})"; sed 's/^/        /' "${SB}/out.log"; }

[ "$(flow_conf "${SB}" MODEL_PATH)" = "$(want "${SB}" "${FLOW_V24_MODEL}")" ] \
    && pass "MODEL_PATH is the V2.4 model" \
    || fail "MODEL_PATH is '$(flow_conf "${SB}" MODEL_PATH)', not the V2.4 model"
[ "$(flow_conf "${SB}" LABELS_PATH)" = "$(want "${SB}" "${FLOW_V24_LABELS}")" ] \
    && pass "LABELS_PATH is the V2.4 labels" \
    || fail "LABELS_PATH is '$(flow_conf "${SB}" LABELS_PATH)', not the V2.4 labels"
[ "$(flow_conf "${SB}" METADATA_MODEL_PATH)" = "$(want "${SB}" "${FLOW_V24_RANGE}")" ] \
    && pass "METADATA_MODEL_PATH is the v2 range filter, live" \
    || fail "METADATA_MODEL_PATH is '$(flow_conf "${SB}" METADATA_MODEL_PATH)', not the v2 range filter"
# The range filter is indexed like the V2.4 labels; a label file of its own
# would be a second, wrong, map (and is the geomodel's mechanism).
[ -z "$(flow_conf "${SB}" METADATA_LABELS_PATH)" ] \
    && pass "no METADATA_LABELS_PATH is set: the filter needs none" \
    || fail "METADATA_LABELS_PATH is live ($(flow_conf "${SB}" METADATA_LABELS_PATH)); the range filter has no labels of its own"

# BirdNET-Pi's defaults, documented and not set: commented, with the numbers.
for line in 'CONFIDENCE=0.70' 'SENSITIVITY=1.25' 'SF_THRESH=0.03' 'OVERLAP=0.0' 'PRIVACY_RANK=10'; do
    if grep -qE "^# ${line}( |$)" "${CFG}"; then
        pass "# ${line} is documented (commented)"
    else
        fail "# ${line} is not in the config as a commented default"
    fi
    if grep -qE "^${line}( |$)" "${CFG}"; then
        fail "${line} is live: a default written live is a setting pinned for ever"
    fi
done

# What the installer asked the binary for, and where it was told to put it.
if grep -qx "INSTALL-MODEL birdnet-v24 dir=$(flow_models_dir "${SB}")" "${SB}/requests.log" \
    && grep -qx "INSTALL-MODEL birdnet-v24-range-v2 dir=$(flow_models_dir "${SB}")" "${SB}/requests.log"; then
    pass "the binary was asked for birdnet-v24 and birdnet-v24-range-v2, into the models directory"
else
    fail "the binary was asked: $(grep INSTALL-MODEL "${SB}/requests.log" | tr '\n' ';')"
fi
# The binary reports every BIRDNET_* variable it does not read, so the
# installer's own settings (the harness always sets BIRDNET_NONINTERACTIVE) must
# not reach it. Also with the classifier chosen by environment.
new_sandbox
BIRDNET_CLASSIFIER=birdnet-v24 BIRDNET_SKIP_MODEL=0 flow_run "${SB}"
if grep -q 'INSTALL-MODEL' "${SB}/requests.log" && ! grep -q '^CHILD-SAW' "${SB}/requests.log"; then
    pass "the binary is run without the installer's own BIRDNET_* settings"
else
    fail "the binary saw: $(grep CHILD-SAW "${SB}/requests.log" | tr '\n' ' ')"
fi
new_sandbox
flow_run "${SB}"
if grep -q '^GET ' "${SB}/requests.log"; then
    fail "the installer fetched something itself for a V2.4 install: $(grep '^GET' "${SB}/requests.log" | tr '\n' ' ')"
else
    pass "the installer fetched nothing itself: no digest, no URL, no labels download of its own"
fi
for f in "${FLOW_V24_MODEL}" "${FLOW_V24_LABELS}" "${FLOW_V24_RANGE}"; do
    [ -s "$(want "${SB}" "${f}")" ] && pass "${f} is on disk" || fail "${f} was not installed"
done
if flow_models_match "${SB}" 'V3|Geomodel'; then
    fail "V3.0 files were fetched for a V2.4 install: $(ls "$(flow_models_dir "${SB}")" | tr '\n' ' ')"
else
    pass "nothing of the V3.0 stack was fetched"
fi

if grep -q 'Classifier: BirdNET V2.4' "${SB}/out.log" && grep -q -- '--classifier birdnet-v3' "${SB}/out.log"; then
    pass "the install says which classifier it chose and how to pick the other"
else
    fail "the install did not name the classifier and the flag for the other one"
fi
if grep -qE '^ +Model: +BirdNET V2\.4' "${SB}/out.log"; then
    pass "the closing summary names the classifier"
else
    fail "the closing summary does not name the classifier"
fi

# ── 2. --classifier birdnet-v3 ──────────────────────────────────────────────
echo "=== 2. counterpart: --classifier birdnet-v3 is the previous behaviour ==="
new_sandbox
flow_run "${SB}" --classifier birdnet-v3
rc=$?
[ "${rc}" -eq 0 ] && pass "the install succeeded" || { fail "the install failed (exit ${rc})"; sed 's/^/        /' "${SB}/out.log"; }
[ "$(flow_conf "${SB}" MODEL_PATH)" = "$(want "${SB}" "${FLOW_V3_MODEL}")" ] \
    && pass "MODEL_PATH is the V3.0 model" || fail "MODEL_PATH is '$(flow_conf "${SB}" MODEL_PATH)'"
[ "$(flow_conf "${SB}" LABELS_PATH)" = "$(want "${SB}" "${FLOW_V3_LABELS}")" ] \
    && pass "LABELS_PATH is the V3.0 labels" || fail "LABELS_PATH is '$(flow_conf "${SB}" LABELS_PATH)'"
[ "$(flow_conf "${SB}" METADATA_MODEL_PATH)" = "$(want "${SB}" "${FLOW_GEO_MODEL}")" ] \
    && pass "METADATA_MODEL_PATH is the geomodel" || fail "METADATA_MODEL_PATH is '$(flow_conf "${SB}" METADATA_MODEL_PATH)'"
[ "$(flow_conf "${SB}" METADATA_LABELS_PATH)" = "$(want "${SB}" "${FLOW_GEO_LABELS}")" ] \
    && pass "METADATA_LABELS_PATH is the geomodel's labels" || fail "METADATA_LABELS_PATH is '$(flow_conf "${SB}" METADATA_LABELS_PATH)'"
if flow_models_match "${SB}" 'birdnet-v24'; then
    fail "V2.4 files were installed for a V3.0 install: $(ls "$(flow_models_dir "${SB}")" | tr '\n' ' ')"
else
    pass "nothing of the V2.4 stack was installed"
fi
if grep -q 'INSTALL-MODEL' "${SB}/requests.log"; then
    fail "the binary was asked to install a model for a V3.0 install: $(grep INSTALL-MODEL "${SB}/requests.log" | tr '\n' ';')"
else
    pass "the binary was not asked to install anything: V3.0 is fetched by the installer, as before"
fi
grep -q 'GET https://github.com/tomtom215/BirdNet-Behavior/releases/download/models-v3.0-preview3/' "${SB}/requests.log" \
    && pass "from the models-v3.0-preview3 release, as before" || fail "the V3.0 origin changed: $(cat "${SB}/requests.log")"
grep -qE '^ +Model: +BirdNET\+ V3\.0 preview' "${SB}/out.log" \
    && pass "the closing summary names the V3.0 preview" || fail "the closing summary does not name the V3.0 preview"

# The V3.0 template is what the previous installer wrote, line for line, except
# for the documented defaults (CONFIDENCE 0.75 -> 0.70 and the OVERLAP wording,
# which were wrong for a BirdNET-Pi-compatible station) and the new PRIVACY_RANK
# line. Compare against a config captured from the previous installer, so
# "unchanged" is a diff and not a belief.
FIXTURE="${HERE}/data/config-v3-written-by-the-previous-installer.conf"
expected="$(sed "s|@SB@|${SB}|g" "${FIXTURE}")"
actual="$(sed 's|^# Generated by install.sh on .*|# Generated by install.sh on 2026-06-01 00:00 UTC|' "$(flow_config "${SB}")")"
delta="$(diff <(printf '%s\n' "${expected}") <(printf '%s\n' "${actual}") || true)"
other="$(grep -E '^[<>]' <<<"${delta}" | grep -vE '^[<>] # (CONFIDENCE|OVERLAP|PRIVACY_RANK)=' || true)"
if [ -z "${delta}" ]; then
    pass "the V3.0 config is identical to the previous installer's"
elif [ -z "${other}" ]; then
    pass "the V3.0 config differs from the previous installer's only in the CONFIDENCE and OVERLAP comments and the new PRIVACY_RANK line"
else
    fail "the V3.0 config differs from the previous installer's beyond the defaults:"
    sed 's/^/        /' <<<"${delta}"
fi

# ── 3. by environment ───────────────────────────────────────────────────────
echo "=== 3. BIRDNET_CLASSIFIER works like the flag, and the flag wins ==="
new_sandbox
BIRDNET_CLASSIFIER=birdnet-v3 flow_run "${SB}"
[ "$(flow_conf "${SB}" MODEL_PATH)" = "$(want "${SB}" "${FLOW_V3_MODEL}")" ] \
    && pass "BIRDNET_CLASSIFIER=birdnet-v3 selects V3.0" || fail "BIRDNET_CLASSIFIER=birdnet-v3 did not select V3.0"
new_sandbox
BIRDNET_CLASSIFIER=birdnet-v3 flow_run "${SB}" --classifier birdnet-v24
[ "$(flow_conf "${SB}" MODEL_PATH)" = "$(want "${SB}" "${FLOW_V24_MODEL}")" ] \
    && pass "--classifier beats BIRDNET_CLASSIFIER" || fail "BIRDNET_CLASSIFIER overrode --classifier"
new_sandbox
flow_run "${SB}" --classifier=birdnet-v3
[ "$(flow_conf "${SB}" MODEL_PATH)" = "$(want "${SB}" "${FLOW_V3_MODEL}")" ] \
    && pass "--classifier=birdnet-v3 (one word) is understood" || fail "--classifier=birdnet-v3 was not understood"

# ── 4. a typo ───────────────────────────────────────────────────────────────
echo "=== 4. an unknown classifier is refused before anything is fetched ==="
for how in "--classifier birdnet-v2.4" "--classifier BIRDNET-V24" "--classifier" "env:birdnet-v4"; do
    new_sandbox
    case "${how}" in
        env:*) BIRDNET_CLASSIFIER="${how#env:}" flow_run "${SB}" ;;
        "--classifier") flow_run "${SB}" --classifier ;;
        *) # shellcheck disable=SC2086  # the word split is the point
           flow_run "${SB}" ${how} ;;
    esac
    rc=$?
    if [ "${rc}" -ne 0 ] && [ ! -s "${SB}/requests.log" ] && [ ! -e "$(flow_config "${SB}")" ] \
        && grep -qE 'birdnet-v24' "${SB}/out.log"; then
        pass "'${how}' stops the install, names the valid ids, and fetches nothing"
    else
        fail "'${how}': exit ${rc}, requests: $(wc -l <"${SB}/requests.log"), config written: $([ -e "$(flow_config "${SB}")" ] && echo yes || echo no)"
        sed 's/^/        /' "${SB}/out.log" | tail -5
    fi
done

# ── 5. a failed range filter ────────────────────────────────────────────────
echo "=== 5. a range filter that cannot be installed warns and leaves the install standing ==="
new_sandbox
FLOW_FAIL_RANGE=1 flow_run "${SB}"
rc=$?
if [ "${rc}" -eq 0 ] && [ -s "$(want "${SB}" "${FLOW_V24_MODEL}")" ] && [ ! -e "$(want "${SB}" "${FLOW_V24_RANGE}")" ]; then
    pass "the classifier installed, the range filter did not, and the install carried on"
else
    fail "exit ${rc}; classifier $([ -s "$(want "${SB}" "${FLOW_V24_MODEL}")" ] && echo present || echo missing); range filter $([ -e "$(want "${SB}" "${FLOW_V24_RANGE}")" ] && echo present || echo absent)"
fi
if grep -qE '^# METADATA_MODEL_PATH=$' "$(flow_config "${SB}")" && [ -z "$(flow_conf "${SB}" METADATA_MODEL_PATH)" ]; then
    pass "METADATA_MODEL_PATH is left as the empty placeholder, not pointed at a file that is not there"
else
    fail "the config names a range filter that is not on disk: '$(flow_conf "${SB}" METADATA_MODEL_PATH)'"
fi
grep -q 'WITHOUT species occurrence filtering' "${SB}/out.log" \
    && pass "and the operator is told filtering is off" || fail "the failure was not reported"

# ── 6. a failed classifier ──────────────────────────────────────────────────
echo "=== 6. counterpart: a classifier that cannot be installed stops the install, and says what to do ==="
for why in FLOW_FAIL_MODEL FLOW_OLD_BINARY; do
    new_sandbox
    case "${why}" in
        FLOW_FAIL_MODEL) FLOW_FAIL_MODEL=1 flow_run "${SB}" ;;
        FLOW_OLD_BINARY) FLOW_OLD_BINARY=1 flow_run "${SB}" ;;
    esac
    rc=$?
    if [ "${rc}" -ne 0 ] && [ ! -e "$(flow_config "${SB}")" ] \
        && grep -q -- '--classifier birdnet-v3' "${SB}/out.log" && grep -q 'BirdNET V2.4 install failed' "${SB}/out.log"; then
        pass "${why}: the install stops (exit ${rc}), writes no config, and offers --classifier birdnet-v3"
    else
        fail "${why}: exit ${rc}; config $([ -e "$(flow_config "${SB}")" ] && echo written || echo absent)"
        sed 's/^/        /' "${SB}/out.log" | tail -6
    fi
done

# ── 7. BIRDNET_SKIP_MODEL ───────────────────────────────────────────────────
echo "=== 7. BIRDNET_SKIP_MODEL=1 installs nothing and says which files to stage ==="
new_sandbox
BIRDNET_SKIP_MODEL=1 flow_run "${SB}"
rc=$?
if [ "${rc}" -eq 0 ] && ! grep -qE '^(GET|INSTALL-MODEL)' "${SB}/requests.log"; then
    pass "nothing was downloaded and the binary was not asked to install anything"
else
    fail "exit ${rc}; requests: $(cat "${SB}/requests.log")"
fi
if grep -q "${FLOW_V24_MODEL}" "${SB}/out.log" && grep -q "${FLOW_V24_LABELS}" "${SB}/out.log" && grep -q "${FLOW_V24_RANGE}" "${SB}/out.log"; then
    pass "the warning names the classifier, its labels and the range filter"
else
    fail "the staging warning does not name all three V2.4 files"
fi
[ "$(flow_conf "${SB}" MODEL_PATH)" = "$(want "${SB}" "${FLOW_V24_MODEL}")" ] \
    && pass "the config still names the V2.4 files, ready for them" || fail "MODEL_PATH is '$(flow_conf "${SB}" MODEL_PATH)'"

# ── 8. the free-space floor ─────────────────────────────────────────────────
echo "=== 8. the free-space floor follows the classifier ==="
# `--install-model` refuses unless the model's size plus a 512 MiB margin is
# free (564 MiB for V2.4's 54 MB model), so V2.4's floor is above that and not
# above its ~85 MB of files; V3.0's 541 MB model needs 900.
run_space() { # $1 = classifier id  $2 = MB free; prints the first line check_disk_space said
    local sb free="$2"
    sb="$(flow_new)"; SANDBOXES+=("${sb}")
    (
        # shellcheck disable=SC1090
        source <(flow_script)
        info() { :; }; success() { echo "OK"; }; warn() { echo "WARN"; }; fatal() { exit 1; }
        free_mb_for_path() { echo "${free}"; }
        MODEL_DIR="${sb}/models"; DATA_DIR="${sb}"
        select_classifier "$1"
        check_disk_space
    ) 2>&1 | head -n 1
}
[ "$(run_space birdnet-v24 700)" = "OK" ]   && pass "700 MB free is enough for BirdNET V2.4"          || fail "700 MB free was called too little for V2.4 (got '$(run_space birdnet-v24 700)')"
[ "$(run_space birdnet-v24 500)" = "WARN" ] && pass "500 MB free is not: the binary would refuse it"   || fail "500 MB free was called enough for V2.4 (got '$(run_space birdnet-v24 500)')"
[ "$(run_space birdnet-v3 700)" = "WARN" ]  && pass "700 MB free is too little for the V3.0 preview"    || fail "700 MB free was called enough for the 541 MB V3.0 model (got '$(run_space birdnet-v3 700)')"

# ── 9. --help ───────────────────────────────────────────────────────────────
echo "=== 9. --help documents the flag and both ids ==="
new_sandbox
flow_run "${SB}" --help
if grep -q -- '--classifier' "${SB}/out.log" && grep -q 'birdnet-v24' "${SB}/out.log" && grep -q 'birdnet-v3' "${SB}/out.log" \
    && grep -q 'default' "${SB}/out.log"; then
    pass "--help lists --classifier with birdnet-v24 (default) and birdnet-v3"
else
    fail "--help does not document --classifier"
fi

echo
if [ "${FAILED}" -ne 0 ]; then
    echo "classifier-default.sh: FAILED"
    exit 1
fi
echo "classifier-default.sh: all pass"
