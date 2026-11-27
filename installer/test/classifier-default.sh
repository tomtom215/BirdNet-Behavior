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
# Every model is installed by the binary (`birdnet-behavior --install-model`),
# which owns its digests, its origins, its retries and the labels file it
# carries (MDL-09). So this drives the real `main`
# (installer/test/lib/install-flow.sh) with a stub binary, and reads what the
# installer did with it: the config, the models directory, the ids it asked the
# binary to install and the retry schedule it handed over, anything it tried to
# fetch itself (nothing), and what it told the operator.
#
#   1. no options            -> V2.4: the binary is asked for the model and for
#                               the range filter, both live in the config;
#                               BirdNET-Pi's defaults documented but commented;
#                               nothing of V3.0 fetched, nothing of V2.4 fetched
#                               by the installer itself.
#   2. --classifier birdnet-v3 -> the binary is asked for the V3.0 model and
#                               the geomodel; the config is the previous
#                               installer's but for the file names.
#   3. the same by environment, and the flag beating it.
#   4. a typo is refused before anything is downloaded.
#   5. the range filter's failure does not fail the install, and is not hidden.
#   6. the classifier's failure (after the binary's own retries, whose schedule
#      the installer hands it: 30 s, then 90 s, or BIRDNET_MODEL_RETRY_WAITS)
#      does NOT stop the install: it finishes, says last and loudly that the
#      station is not detecting and how to fix it, and exits 75 (OOB-20,
#      decision 7 of 2026-11-25).
#   6b. a local model file (--local-model / BIRDNET_LOCAL_MODEL) is handed to
#      the binary's check and installed, or refused (exit 75, nothing at the
#      final name), for V3.0 as for V2.4; a path that is not a file stops the
#      run before anything is installed.
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
echo "=== 2. counterpart: --classifier birdnet-v3 runs V3.0 and its geomodel ==="
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
if grep -qx "INSTALL-MODEL birdnet-v3 dir=$(flow_models_dir "${SB}")" "${SB}/requests.log" \
    && grep -qx "INSTALL-MODEL birdnet-v3-geomodel dir=$(flow_models_dir "${SB}")" "${SB}/requests.log" \
    && ! grep -q '^GET ' "${SB}/requests.log"; then
    pass "the binary was asked for birdnet-v3 and birdnet-v3-geomodel, and the installer fetched nothing itself"
else
    fail "requests: $(tr '\n' ';' <"${SB}/requests.log")"
fi
grep -qE '^ +Model: +BirdNET\+ V3\.0 preview' "${SB}/out.log" \
    && pass "the closing summary names the V3.0 preview" || fail "the closing summary does not name the V3.0 preview"

# The V3.0 template is what the previous installer wrote, line for line, except
# for the model files' names (the binary's, MDL-09: compared after mapping the
# previous installer's names onto them), the documented defaults (CONFIDENCE 0.75 -> 0.70 and the OVERLAP wording,
# which were wrong for a BirdNET-Pi-compatible station), the new PRIVACY_RANK
# line, and the RECORDING_SCHEDULE comment's clock (fixed hours were documented
# as UTC; the scheduler has always used local time, OOB-04). Compare against a config captured from the previous installer, so
# "unchanged" is a diff and not a belief.
FIXTURE="${HERE}/data/config-v3-written-by-the-previous-installer.conf"
expected="$(sed -e "s|@SB@|${SB}|g" -e "s|${FLOW_V3_LEGACY_MODEL}|${FLOW_V3_MODEL}|" \
    -e "s|${FLOW_V3_LEGACY_LABELS}|${FLOW_V3_LABELS}|" -e "s|${FLOW_GEO_LEGACY_MODEL}|${FLOW_GEO_MODEL}|" \
    -e "s|${FLOW_GEO_LEGACY_LABELS}|${FLOW_GEO_LABELS}|" "${FIXTURE}")"
actual="$(sed 's|^# Generated by install.sh on .*|# Generated by install.sh on 2026-06-01 00:00 UTC|' "$(flow_config "${SB}")")"
delta="$(diff <(printf '%s\n' "${expected}") <(printf '%s\n' "${actual}") || true)"
other="$(grep -E '^[<>]' <<<"${delta}" | grep -vE '^[<>] # (CONFIDENCE|OVERLAP|PRIVACY_RANK)=' \
    | grep -vxF \
        -e '< #                              # coordinates above. Fixed hours are evaluated in' \
        -e '< #                              # UTC, not local time. Also settable in the web UI' \
        -e '> #                              # coordinates above. Fixed hours are in the' \
        -e "> #                              # station's local time. Also settable in the web UI" \
    || true)"
if [ -z "${delta}" ]; then
    pass "the V3.0 config is identical to the previous installer's"
elif [ -z "${other}" ]; then
    pass "the V3.0 config differs from the previous installer's only in the CONFIDENCE and OVERLAP comments, the new PRIVACY_RANK line and the fixed-hours clock"
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
echo "=== 6. a classifier that cannot be installed: the install finishes, says so last, and exits 75 ==="
for why in FLOW_FAIL_MODEL FLOW_OLD_BINARY; do
    new_sandbox
    case "${why}" in
        FLOW_FAIL_MODEL) FLOW_FAIL_MODEL=1 flow_run "${SB}" ;;
        FLOW_OLD_BINARY) FLOW_OLD_BINARY=1 flow_run "${SB}" ;;
    esac
    rc=$?
    if [ "${rc}" -eq 75 ]; then
        pass "${why}: the run exits 75 (installed, not detecting)"
    else
        fail "${why}: exit ${rc}, not 75"
        sed 's/^/        /' "${SB}/out.log" | tail -8
    fi
    [ "$(flow_conf "${SB}" MODEL_PATH)" = "$(want "${SB}" "${FLOW_V24_MODEL}")" ] \
        && pass "${why}: the config is written and still names the V2.4 model, so a re-run keeps V2.4" \
        || fail "${why}: MODEL_PATH is '$(flow_conf "${SB}" MODEL_PATH)'"
    [ ! -e "$(want "${SB}" "${FLOW_V24_MODEL}")" ] \
        && pass "${why}: nothing is at the model's name" || fail "${why}: a model file appeared"
    # The binary retries; the installer asks once and hands it the schedule.
    tries="$(grep -c '^INSTALL-MODEL birdnet-v24 ' "${SB}/requests.log")"
    [ "${tries}" -eq 1 ] && pass "${why}: the installer asks the binary once" || fail "${why}: ${tries} calls, not 1"
    if grep -qx 'RETRY-WAITS birdnet-v24 30 90' "${SB}/requests.log" && ! grep -q '^SLEEP' "${SB}/requests.log"; then
        pass "${why}: with the retry schedule 30 90, and no waiting of its own"
    else
        fail "${why}: schedule '$(grep '^RETRY-WAITS birdnet-v24' "${SB}/requests.log")', sleeps '$(grep -c '^SLEEP' "${SB}/requests.log")'"
    fi
    if grep -q 'Installation complete!' "${SB}/out.log"; then
        fail "${why}: the summary says 'Installation complete!' over a station that cannot detect"
    else
        pass "${why}: the summary does not say 'Installation complete!'"
    fi
    tail_text="$(sed -n '/For troubleshooting/,$p' "${SB}/out.log")"
    if grep -q 'NOT DETECTING BIRDS' <<<"${tail_text}" && grep -q 'install.sh repair' <<<"${tail_text}" \
        && grep -q -- '--local-model' <<<"${tail_text}" && grep -q 'status 75' <<<"${tail_text}"; then
        pass "${why}: the run ends saying the station is not detecting, how to install the model, and the status"
    else
        fail "${why}: the end of the run does not say it: $(tail -12 "${SB}/out.log" | tr '\n' '|')"
    fi
done
# The operator's schedule reaches the binary as given — every model's, the
# filter's too, which the installer used to fetch once with no retry at all.
new_sandbox
BIRDNET_MODEL_RETRY_WAITS="" flow_run "${SB}"
if grep -qx 'RETRY-WAITS birdnet-v24 ' "${SB}/requests.log" && grep -qx 'RETRY-WAITS birdnet-v24-range-v2 ' "${SB}/requests.log"; then
    pass "BIRDNET_MODEL_RETRY_WAITS=\"\" reaches the binary for the classifier and the filter"
else
    fail "the schedule handed over: $(grep '^RETRY-WAITS' "${SB}/requests.log" | tr '\n' ';')"
fi
new_sandbox
FLOW_FAIL_V3_MODEL=1 flow_run "${SB}" --classifier birdnet-v3
rc=$?
if [ "${rc}" -eq 75 ] && [ "$(flow_conf "${SB}" MODEL_PATH)" = "$(want "${SB}" "${FLOW_V3_MODEL}")" ] \
    && grep -q 'NOT DETECTING BIRDS' "${SB}/out.log" && ! grep -q '^GET ' "${SB}/requests.log"; then
    pass "--classifier birdnet-v3 with the model unreachable: the install finished, exit 75"
else
    fail "--classifier birdnet-v3 with the model unreachable: exit ${rc}; requests: $(tr '\n' ';' <"${SB}/requests.log")"
    sed 's/^/        /' "${SB}/out.log" | tail -6
fi

echo "=== 6b. a local model file is checked like a download: installed when it matches, refused when not ==="
for how in env flag; do
    new_sandbox
    printf '%s' "${FLOW_BODY_V24_MODEL}" >"${SB}/my-model.onnx"
    case "${how}" in
        env)  BIRDNET_LOCAL_MODEL="${SB}/my-model.onnx" flow_run "${SB}" ;;
        flag) flow_run "${SB}" --local-model "${SB}/my-model.onnx" ;;
    esac
    rc=$?
    if [ "${rc}" -eq 0 ] && grep -qx "INSTALL-MODEL birdnet-v24 dir=$(flow_models_dir "${SB}") from=${SB}/my-model.onnx" "${SB}/requests.log" \
        && [ -s "$(want "${SB}" "${FLOW_V24_MODEL}")" ] && ! grep -q '^CHILD-SAW' "${SB}/requests.log"; then
        pass "${how}: the file is handed to the binary's check (--from-file) and installed; exit 0"
    else
        fail "${how}: exit ${rc}; requests: $(tr '\n' ';' <"${SB}/requests.log")"
        sed 's/^/        /' "${SB}/out.log" | tail -6
    fi
done
new_sandbox
printf '%s' "not the model" >"${SB}/wrong.onnx"
BIRDNET_LOCAL_MODEL="${SB}/wrong.onnx" flow_run "${SB}"
rc=$?
tries="$(grep -c '^INSTALL-MODEL birdnet-v24 ' "${SB}/requests.log")"
if [ "${rc}" -eq 75 ] && [ "${tries}" -eq 1 ] && [ ! -e "$(want "${SB}" "${FLOW_V24_MODEL}")" ] \
    && grep -q 'was refused' "${SB}/out.log" && grep -q 'NOT DETECTING BIRDS' "${SB}/out.log"; then
    pass "a file that does not match is refused once (no retries), nothing is installed, exit 75"
else
    fail "a wrong local file: exit ${rc}, ${tries} attempts, model $([ -e "$(want "${SB}" "${FLOW_V24_MODEL}")" ] && echo present || echo absent)"
fi
new_sandbox
BIRDNET_LOCAL_MODEL="${SB}/no-such-file.onnx" flow_run "${SB}"
rc=$?
if [ "${rc}" -ne 0 ] && [ "${rc}" -ne 75 ] && [ ! -s "${SB}/requests.log" ] && [ ! -e "$(flow_config "${SB}")" ]; then
    pass "a local model path that is not a file stops the run (exit ${rc}) before anything is installed"
else
    fail "a missing local file: exit ${rc}; config $([ -e "$(flow_config "${SB}")" ] && echo written || echo absent)"
fi
# V3.0: the installer holds the pin and checks the file itself.
new_sandbox
printf '%s' "${FLOW_BODY_V3_MODEL}" >"${SB}/v3.onnx"
flow_run "${SB}" --classifier birdnet-v3 --local-model "${SB}/v3.onnx"
rc=$?
if [ "${rc}" -eq 0 ] && grep -qx "INSTALL-MODEL birdnet-v3 dir=$(flow_models_dir "${SB}") from=${SB}/v3.onnx" "${SB}/requests.log" \
    && [ "$(cat "$(want "${SB}" "${FLOW_V3_MODEL}")" 2>/dev/null)" = "${FLOW_BODY_V3_MODEL}" ]; then
    pass "--classifier birdnet-v3 --local-model: the matching file is installed and the model is not downloaded"
else
    fail "V3.0 local model: exit ${rc}; requests: $(tr '\n' ';' <"${SB}/requests.log")"
fi
new_sandbox
printf '%s' "not the V3.0 model" >"${SB}/v3.onnx"
flow_run "${SB}" --classifier birdnet-v3 --local-model "${SB}/v3.onnx"
rc=$?
if [ "${rc}" -eq 75 ] && [ ! -e "$(want "${SB}" "${FLOW_V3_MODEL}")" ] \
    && grep -q 'sha256 mismatch' "${SB}/out.log"; then
    pass "--classifier birdnet-v3: a file that does not match the pin is refused and leaves nothing behind; exit 75"
else
    fail "V3.0 wrong local model: exit ${rc}; models: $(ls "$(flow_models_dir "${SB}")" | tr '\n' ' ')"
fi

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
# MDL-27: the station repairs the files its config names, so it fetches these
# itself. The warning used to say it would "detect NOTHING until you stage it".
if grep -q "The station fetches it itself" "${SB}/out.log" && grep -q "BIRDNET_OFFLINE=1" "${SB}/out.log" \
    && ! grep -q "detect NOTHING until you stage it" "${SB}/out.log"; then
    pass "the warning says the station fetches the files itself, and how to stop it"
else
    fail "the staging warning does not say the station fetches the files itself (MDL-27)"
fi

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
