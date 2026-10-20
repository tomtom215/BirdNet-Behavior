#!/usr/bin/env bash
# installer/test/classifier-docker.sh — which classifier does the CONTAINER run?
#
# The same decision as the bare-metal installer, made by docker/entrypoint.sh:
# a new data volume gets BirdNET V2.4 and its range filter; the BirdNET+ V3.0
# preview is `BIRDNET_CLASSIFIER=birdnet-v3`; and a volume that already holds the
# V3.0 model and has no BIRDNET_CLASSIFIER set keeps running V3.0, with one log
# line saying so and how to change it — because an image upgrade must never swap
# the network under a running station's history.
#
# BirdNET V2.4 is installed by the binary (`birdnet-behavior --install-model`),
# which owns its digests, its origins and the labels file it carries; the
# entrypoint has none of that, so the binary is stubbed here (BNB_BINARY, the
# same hook BNB_STRIP_LIB gives the blank-environment scrubber). BirdNET+ V3.0 is
# still downloaded by the entrypoint and its download functions are recorded.
#
# Docker is not available to CI's hermetic suite, so this does not run a
# container. It does the two things that can be done without one:
#
#   Part 1 sources the entrypoint's own functions (select_classifier,
#          acquire_classifier_models, setup_occurrence_filter, ...) with the
#          download functions replaced by recorders, and reads what each
#          scenario asked for and exported.
#   Part 2 runs the WHOLE entrypoint script, byte for byte, with a stub binary
#          that plays both parts the real one plays here (installing a model,
#          then being the daemon), and reads the log and the environment the
#          daemon would have started with.
#
# What neither can show: that the image builds, that the real binary can reach
# its origins from inside a container, that Compose passes the variable through.
# See the report that accompanied this change for what was and was not run.
#
# Usage: installer/test/classifier-docker.sh
# Needs bash + coreutils. No docker, no root, no network.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
ENTRYPOINT="${REPO_ROOT}/docker/entrypoint.sh"
FAILED=0
pass() { printf '  PASS  %s\n' "$*"; }
fail() { printf '  FAIL  %s\n' "$*"; FAILED=1; }

WORK="$(mktemp -d)"
trap 'rm -rf "${WORK}"' EXIT

# The file names and the V3.0 volume marker, as the contract says.
V24_MODEL="birdnet-v24.onnx"
V24_LABELS="birdnet-v24_labels.txt"
V24_RANGE="birdnet-v24-range-v2.onnx"
V3_MODEL="BirdNET+_V3.0-preview3_Global_11K_FP32.onnx"
V3_LABELS="BirdNET+_V3.0-preview3_Global_11K_Labels.csv"
GEO_MODEL="BirdNET+_Geomodel_V3.0.2_Global_12K_FP32.onnx"
GEO_LABELS="BirdNET+_Geomodel_V3.0.2_Global_12K_Labels.txt"

# ── Part 1: the functions ───────────────────────────────────────────────────

# Run the entrypoint's classifier-and-filter logic in a fresh volume.
#
#   run_logic <volume-contents...>   (names of files to pre-create in /model;
#                                     a name prefixed "empty:" is made zero-byte)
#
# Environment for the scenario is whatever the caller exported/prefixed.
# Writes ${WORK}/logic.out (everything logged), ${WORK}/logic.calls (what the
# install/download functions were asked for) and ${WORK}/logic.env (what was
# exported).
run_logic() {
    local vol="${WORK}/volume.$RANDOM"
    mkdir -p "${vol}"
    local f
    for f in "$@"; do
        case "${f}" in
            empty:*) : >"${vol}/${f#empty:}" ;;
            *) echo content >"${vol}/${f}" ;;
        esac
    done
    : >"${WORK}/logic.calls"
    rm -f "${WORK}/logic.env"
    (
        set -uo pipefail
        log()  { echo "[birdnet] $*"; }
        warn() { echo "[birdnet] WARNING: $*"; }
        die()  { echo "[birdnet] ERROR: $*"; exit 1; }
        rule() { :; }

        MODEL_DIR="${vol}"
        # The V3.0 pins and the names, taken from the shipping file.
        # shellcheck disable=SC1090
        source <(grep -E '^(V24_[A-Z0-9_]*|V3_[A-Z0-9_]*|ZENODO_BASE|GEOMODEL_[A-Z0-9_]*)=' "${ENTRYPOINT}")
        # shellcheck disable=SC1090
        source <(sed -n '/^select_classifier()/,/^}/p;/^acquire_default_models()/,/^}/p;/^acquire_v24_models()/,/^}/p;/^acquire_classifier_models()/,/^}/p;/^setup_geomodel()/,/^}/p;/^setup_range_filter()/,/^}/p;/^setup_occurrence_filter()/,/^}/p' "${ENTRYPOINT}")

        # The V3.0 download recorders: note the call, make the file, succeed.
        ensure_model_file() { echo "v3-model $1 | primary=$2 | fallback=$3 | sha=$4" >>"${WORK}/logic.calls"; echo v3 >"$1"; }
        ensure_geomodel_file() { echo "geomodel $1 | sha=$3" >>"${WORK}/logic.calls"; echo geo >"$1"; }
        # The binary: note the id, write what the real one writes.
        install_with_binary() {
            echo "binary $1" >>"${WORK}/logic.calls"
            case "$1" in
                birdnet-v24) echo m >"${MODEL_DIR}/${V24_MODEL_FILE}"; echo l >"${MODEL_DIR}/${V24_LABELS_FILE}" ;;
                birdnet-v24-range-v2) [ "${FAIL_RANGE:-0}" = 1 ] && return 1; echo r >"${MODEL_DIR}/${V24_RANGE_FILE}" ;;
                *) return 1 ;;
            esac
        }

        # Same order as the entrypoint's top level.
        select_classifier
        MODEL_DEFAULT="${MODEL_DIR}/${MODEL_FILE}"
        LABELS_DEFAULT="${MODEL_DIR}/${LABELS_FILE}"
        : "${BIRDNET_MODEL:=${MODEL_DEFAULT}}"
        : "${BIRDNET_LABELS:=${LABELS_DEFAULT}}"
        export BIRDNET_MODEL BIRDNET_LABELS
        # shellcheck disable=SC2034  # read by the sourced setup_* functions
        {
            GEOMODEL_USER_SET=0
            [ -n "${BIRDNET_METADATA_MODEL:-}" ] && GEOMODEL_USER_SET=1
        }
        acquire_classifier_models
        setup_occurrence_filter
        {
            echo "CLASSIFIER=${CLASSIFIER}"
            echo "BIRDNET_MODEL=${BIRDNET_MODEL}"
            echo "BIRDNET_LABELS=${BIRDNET_LABELS}"
            echo "BIRDNET_METADATA_MODEL=${BIRDNET_METADATA_MODEL:-}"
            echo "BIRDNET_METADATA_LABELS=${BIRDNET_METADATA_LABELS:-}"
            echo "VOLUME=${vol}"
        } >"${WORK}/logic.env"
    ) >"${WORK}/logic.out" 2>&1
}
envval() { sed -n "s/^$1=//p" "${WORK}/logic.env"; }
classifier_lines() { grep -c '\[birdnet\] Classifier:' "${WORK}/logic.out" || true; }

# the "never mixed" rule, read off what was exported
assert_unmixed() { # $1 label
    local m f
    m="$(envval BIRDNET_MODEL)"; f="$(envval BIRDNET_METADATA_MODEL)"
    case "${m##*/}" in
        "${V24_MODEL}")
            if [ -z "${f}" ] || [ "${f##*/}" = "${V24_RANGE}" ]; then pass "$1: V2.4 is never given the geomodel"
            else fail "$1: V2.4 was given '${f##*/}'"; fi
            [ -z "$(envval BIRDNET_METADATA_LABELS)" ] || fail "$1: V2.4 was given a metadata labels file"
            ;;
        "${V3_MODEL}")
            if [ -z "${f}" ] || [ "${f##*/}" = "${GEO_MODEL}" ]; then pass "$1: V3.0 is never given the range filter"
            else fail "$1: V3.0 was given '${f##*/}'"; fi
            ;;
    esac
}

echo "=== 1. a new volume runs BirdNET V2.4 and its range filter, both installed by the binary ==="
run_logic
rc=$?
V="$(envval VOLUME)"
[ "${rc}" -eq 0 ] && pass "the entrypoint logic ran" || { fail "it failed (exit ${rc})"; sed 's/^/        /' "${WORK}/logic.out"; }
[ "$(envval CLASSIFIER)" = "birdnet-v24" ] && pass "CLASSIFIER is birdnet-v24" || fail "CLASSIFIER is '$(envval CLASSIFIER)'"
[ "$(envval BIRDNET_MODEL)" = "${V}/${V24_MODEL}" ] && pass "BIRDNET_MODEL is the V2.4 model" || fail "BIRDNET_MODEL is '$(envval BIRDNET_MODEL)'"
[ "$(envval BIRDNET_LABELS)" = "${V}/${V24_LABELS}" ] && pass "BIRDNET_LABELS is the V2.4 labels" || fail "BIRDNET_LABELS is '$(envval BIRDNET_LABELS)'"
[ "$(envval BIRDNET_METADATA_MODEL)" = "${V}/${V24_RANGE}" ] && pass "BIRDNET_METADATA_MODEL is the v2 range filter" || fail "BIRDNET_METADATA_MODEL is '$(envval BIRDNET_METADATA_MODEL)'"
[ -z "$(envval BIRDNET_METADATA_LABELS)" ] && pass "no BIRDNET_METADATA_LABELS: the filter needs none" || fail "BIRDNET_METADATA_LABELS is set"
if [ "$(grep -c '^binary ' "${WORK}/logic.calls")" = 2 ] && grep -qx 'binary birdnet-v24' "${WORK}/logic.calls" && grep -qx 'binary birdnet-v24-range-v2' "${WORK}/logic.calls"; then
    pass "the binary was asked for birdnet-v24 and for birdnet-v24-range-v2, once each"
else
    fail "the calls: $(tr '\n' ';' <"${WORK}/logic.calls")"
fi
if grep -qE '^(v3-model|geomodel) ' "${WORK}/logic.calls"; then
    fail "the entrypoint downloaded something itself for a V2.4 volume: $(grep -E '^(v3-model|geomodel)' "${WORK}/logic.calls" | head -2)"
else
    pass "the entrypoint downloaded nothing itself: no V2.4 digest or URL of its own"
fi
[ "$(classifier_lines)" = 1 ] && grep -q 'Classifier: BirdNET V2.4' "${WORK}/logic.out" \
    && pass "exactly one 'Classifier:' line, naming V2.4" || fail "the Classifier line(s): $(grep 'Classifier:' "${WORK}/logic.out")"
grep 'Classifier:' "${WORK}/logic.out" | grep -q 'BIRDNET_CLASSIFIER=birdnet-v3' && pass "...which says how to choose the other" || fail "the line does not say how to choose V3.0"
assert_unmixed "new volume"

echo "=== 1b. the next start, with the files in the volume, installs nothing ==="
run_logic "${V24_MODEL}" "${V24_LABELS}" "${V24_RANGE}"
V="$(envval VOLUME)"
if [ ! -s "${WORK}/logic.calls" ] && [ "$(envval BIRDNET_METADATA_MODEL)" = "${V}/${V24_RANGE}" ] && [ "$(envval BIRDNET_MODEL)" = "${V}/${V24_MODEL}" ]; then
    pass "no call to the binary, and the range filter is still configured"
else
    fail "a start with a complete volume made calls: $(tr '\n' ';' <"${WORK}/logic.calls")"
fi
echo "=== 1c. counterpart: a missing or empty file IS installed ==="
run_logic "${V24_LABELS}" "${V24_RANGE}"
if grep -qx 'binary birdnet-v24' "${WORK}/logic.calls" && ! grep -qx 'binary birdnet-v24-range-v2' "${WORK}/logic.calls"; then
    pass "a volume without the model gets the model (and keeps its range filter)"
else
    fail "calls: $(tr '\n' ';' <"${WORK}/logic.calls")"
fi
run_logic "empty:${V24_MODEL}" "${V24_LABELS}" "${V24_RANGE}"
if grep -qx 'binary birdnet-v24' "${WORK}/logic.calls"; then
    pass "a zero-byte model is treated as missing"
else
    fail "a zero-byte model was trusted: $(tr '\n' ';' <"${WORK}/logic.calls")"
fi

echo "=== 2. counterpart: a volume that already holds the V3.0 model keeps running it ==="
run_logic "${V3_MODEL}" "${V3_LABELS}"
V="$(envval VOLUME)"
[ "$(envval CLASSIFIER)" = "birdnet-v3" ] && pass "CLASSIFIER stays birdnet-v3" || fail "CLASSIFIER is '$(envval CLASSIFIER)'"
[ "$(envval BIRDNET_MODEL)" = "${V}/${V3_MODEL}" ] && pass "BIRDNET_MODEL is still the V3.0 model" || fail "BIRDNET_MODEL is '$(envval BIRDNET_MODEL)'"
[ "$(envval BIRDNET_METADATA_MODEL)" = "${V}/${GEO_MODEL}" ] && [ "$(envval BIRDNET_METADATA_LABELS)" = "${V}/${GEO_LABELS}" ] \
    && pass "and the geomodel with its labels is still the filter" || fail "the V3.0 filter changed: '$(envval BIRDNET_METADATA_MODEL)'"
if grep -q '^binary ' "${WORK}/logic.calls"; then fail "the V2.4 installer was run for a V3.0 volume: $(grep '^binary' "${WORK}/logic.calls")"; else pass "the binary was not asked to install V2.4"; fi
if [ "$(classifier_lines)" = 1 ] && grep 'Classifier:' "${WORK}/logic.out" | grep -q 'keeps running it' \
    && grep 'Classifier:' "${WORK}/logic.out" | grep -q 'BIRDNET_CLASSIFIER=birdnet-v24'; then
    pass "exactly one log line says it kept V3.0 and how to move to V2.4"
else
    fail "the notice: $(grep 'Classifier:' "${WORK}/logic.out")"
fi
assert_unmixed "V3.0 volume"

echo "=== 3. BIRDNET_CLASSIFIER=birdnet-v3 on a new volume is V3.0, downloaded as before ==="
BIRDNET_CLASSIFIER=birdnet-v3 run_logic
V="$(envval VOLUME)"
[ "$(envval BIRDNET_MODEL)" = "${V}/${V3_MODEL}" ] && [ "$(envval BIRDNET_METADATA_MODEL)" = "${V}/${GEO_MODEL}" ] \
    && pass "the V3.0 model and the geomodel" || fail "model '$(envval BIRDNET_MODEL)' filter '$(envval BIRDNET_METADATA_MODEL)'"
if grep -q '^binary ' "${WORK}/logic.calls"; then fail "the binary was asked to install V2.4: $(grep '^binary' "${WORK}/logic.calls")"; else pass "the binary was not asked to install anything"; fi
grep -q "primary=https://github.com/tomtom215/BirdNet-Behavior/releases/download/models-v3.0-preview3/" "${WORK}/logic.calls" \
    && grep -q "fallback=https://zenodo.org/api/records/18247420/files/" "${WORK}/logic.calls" \
    && pass "from the V3.0 release, falling back to Zenodo, as before" || fail "the V3.0 origins changed: $(grep '^v3-model' "${WORK}/logic.calls")"
grep 'Classifier:' "${WORK}/logic.out" | grep -q 'BIRDNET_CLASSIFIER=birdnet-v3' && pass "the log says the choice came from BIRDNET_CLASSIFIER" || fail "the log does not credit BIRDNET_CLASSIFIER"
assert_unmixed "explicit V3.0"

echo "=== 4. BIRDNET_CLASSIFIER=birdnet-v24 on a V3.0 volume moves it, because someone asked ==="
BIRDNET_CLASSIFIER=birdnet-v24 run_logic "${V3_MODEL}" "${V3_LABELS}"
V="$(envval VOLUME)"
[ "$(envval BIRDNET_MODEL)" = "${V}/${V24_MODEL}" ] && [ "$(envval BIRDNET_METADATA_MODEL)" = "${V}/${V24_RANGE}" ] \
    && pass "the V2.4 model and the range filter" || fail "model '$(envval BIRDNET_MODEL)' filter '$(envval BIRDNET_METADATA_MODEL)'"
assert_unmixed "explicit V2.4 on a V3.0 volume"

echo "=== 5. a value that is neither id stops the container, naming the two that are ==="
BIRDNET_CLASSIFIER=birdnet-v2.4 run_logic
rc=$?
if [ "${rc}" -ne 0 ] && grep -q 'birdnet-v24' "${WORK}/logic.out" && grep -q 'birdnet-v3' "${WORK}/logic.out" && [ ! -s "${WORK}/logic.calls" ]; then
    pass "exit ${rc}, both valid ids named, nothing installed or fetched"
else
    fail "exit ${rc}; output: $(cat "${WORK}/logic.out")"
fi

echo "=== 6. a model of the operator's own keeps the V3.0 defaults it always had ==="
BIRDNET_MODEL=/data/model/my-own.onnx run_logic
V="$(envval VOLUME)"
if [ "$(envval BIRDNET_MODEL)" = "/data/model/my-own.onnx" ] && [ "$(envval CLASSIFIER)" = "birdnet-v3" ] \
    && [ "$(envval BIRDNET_METADATA_MODEL)" = "${V}/${GEO_MODEL}" ] && ! grep -q "^v3-model /data/model/my-own.onnx" "${WORK}/logic.calls"; then
    pass "their model is untouched and unverified; labels and the geomodel default as before"
else
    fail "own-model handling changed: classifier '$(envval CLASSIFIER)', filter '$(envval BIRDNET_METADATA_MODEL)'"
fi

echo "=== 6b. BIRDNET_MODEL naming the V2.4 file is V2.4 ==="
# select_classifier compares BIRDNET_MODEL with ${MODEL_DIR}/<file>, and the
# helper picks MODEL_DIR itself; give it a fixed one to compare against.
(
    set -uo pipefail
    log()  { echo "[birdnet] $*"; }
    warn() { :; }
    die()  { echo "[birdnet] ERROR: $*"; exit 1; }
    MODEL_DIR="${WORK}/fixed-model-dir"
    mkdir -p "${MODEL_DIR}"
    # shellcheck disable=SC1090
    source <(grep -E '^(V24_[A-Z0-9_]*|V3_[A-Z0-9_]*|ZENODO_BASE)=' "${ENTRYPOINT}")
    # shellcheck disable=SC1090
    source <(sed -n '/^select_classifier()/,/^}/p' "${ENTRYPOINT}")
    BIRDNET_MODEL="${MODEL_DIR}/${V24_MODEL}"
    select_classifier >/dev/null
    echo "${CLASSIFIER}"
) >"${WORK}/6b.out" 2>&1
[ "$(cat "${WORK}/6b.out")" = "birdnet-v24" ] && pass "BIRDNET_MODEL=<model dir>/${V24_MODEL} selects V2.4" || fail "selected '$(cat "${WORK}/6b.out")'"

echo "=== 7. an operator-set BIRDNET_METADATA_MODEL is theirs ==="
BIRDNET_METADATA_MODEL=/data/mine/filter.onnx run_logic
if [ "$(envval BIRDNET_METADATA_MODEL)" = "/data/mine/filter.onnx" ] && ! grep -qx 'binary birdnet-v24-range-v2' "${WORK}/logic.calls"; then
    pass "the V2.4 range filter was not installed over it"
else
    fail "BIRDNET_METADATA_MODEL was replaced: '$(envval BIRDNET_METADATA_MODEL)'; calls: $(tr '\n' ';' <"${WORK}/logic.calls")"
fi

echo "=== 8. BIRDNET_METADATA_LABELS beside the range filter is flagged, not silently paired ==="
BIRDNET_METADATA_LABELS=/data/mine/geo-labels.txt run_logic
if grep -q 'WARNING: BIRDNET_METADATA_LABELS is set, but the V2.4 range filter' "${WORK}/logic.out"; then
    pass "a warning says the V2.4 range filter needs no labels file"
else
    fail "no warning for a label file set beside the V2.4 range filter"
fi

echo "=== 9. BIRDNET_SKIP_MODEL_DOWNLOAD=1 installs nothing and still says which classifier ==="
BIRDNET_SKIP_MODEL_DOWNLOAD=1 run_logic
V="$(envval VOLUME)"
if [ ! -s "${WORK}/logic.calls" ] && [ "$(envval BIRDNET_MODEL)" = "${V}/${V24_MODEL}" ] && [ -z "$(envval BIRDNET_METADATA_MODEL)" ] && [ "$(classifier_lines)" = 1 ]; then
    pass "no install, the V2.4 paths are set, no filter is claimed, one Classifier line"
else
    fail "skip handling: calls=$(wc -l <"${WORK}/logic.calls") model=$(envval BIRDNET_MODEL) filter='$(envval BIRDNET_METADATA_MODEL)'"
fi

echo "=== 10. the range filter failing leaves a working container without it ==="
FAIL_RANGE=1 run_logic
rc=$?
V="$(envval VOLUME)"
if [ "${rc}" -eq 0 ] && [ "$(envval BIRDNET_MODEL)" = "${V}/${V24_MODEL}" ] && [ -z "$(envval BIRDNET_METADATA_MODEL)" ] \
    && grep -q 'species occurrence filtering is OFF' "${WORK}/logic.out"; then
    pass "the classifier is configured, no filter is claimed, and the log says filtering is off"
else
    fail "exit ${rc}; filter '$(envval BIRDNET_METADATA_MODEL)'; $(grep -c 'filtering is OFF' "${WORK}/logic.out") notice(s)"
fi

# ── Part 2: the whole script ────────────────────────────────────────────────
echo "=== 11. the real entrypoint, end to end ==="
# BNB_BINARY is a stub that plays both parts the real binary plays: with
# --install-model it writes what the real one writes (the same names) and
# records the id; with anything else it is the daemon and prints what it was
# started with.
STUB="${WORK}/binary-stub"
cat >"${STUB}" <<'SH'
#!/bin/sh
id=""
for a in "$@"; do
    if [ "${id}" = "?" ]; then id="${a}"; fi
    [ "${a}" = "--install-model" ] && id="?"
done
if [ -n "${id}" ]; then
    # The daemon would report any BIRDNET_* variable it does not read; so would
    # this install, and it must not be handed BIRDNET_CLASSIFIER.
    [ -n "${BIRDNET_CLASSIFIER:-}" ] && echo "STUB-SAW-BIRDNET_CLASSIFIER" >>"${STUB_LOG}"
    echo "STUB-INSTALL: ${id} (dir=${BIRDNET_MODEL_DIR})" >>"${STUB_LOG}"
    [ "${STUB_FAIL:-}" = "${id}" ] && { echo "Install failed: network error" >&2; exit 1; }
    case "${id}" in
        birdnet-v24) echo model >"${BIRDNET_MODEL_DIR}/birdnet-v24.onnx"; echo labels >"${BIRDNET_MODEL_DIR}/birdnet-v24_labels.txt" ;;
        birdnet-v24-range-v2) echo range >"${BIRDNET_MODEL_DIR}/birdnet-v24-range-v2.onnx" ;;
        *) echo "Install failed: no classifier called ${id}" >&2; exit 1 ;;
    esac
    exit 0
fi
echo "DAEMON-ARGS: $*"
env | sort | grep -E '^BIRDNET_' | sed 's/^/DAEMON-ENV: /'
SH
chmod +x "${STUB}"

run_entrypoint() { # volume dir, then extra env as NAME=VALUE...
    local vol="$1"; shift
    mkdir -p "${vol}"
    : >"${WORK}/stub.log"
    env -i PATH="${PATH}" HOME="${WORK}" TZ=UTC STUB_LOG="${WORK}/stub.log" \
        BNB_STRIP_LIB="${REPO_ROOT}/docker/strip-blank-env.sh" BNB_BINARY="${STUB}" \
        BIRDNET_MODEL_DIR="${vol}" BIRDNET_ALSA_DEVICE=hw:0 "$@" \
        sh "${ENTRYPOINT}" --some-arg >"${WORK}/e2e.out" 2>&1
}
denv() { sed -n "s/^DAEMON-ENV: $1=//p" "${WORK}/e2e.out"; }

vol="${WORK}/e2e-new"
run_entrypoint "${vol}"; rc=$?
[ "${rc}" -eq 0 ] && pass "a new volume: the entrypoint installed V2.4 and ran on to the daemon" || { fail "exit ${rc}"; sed 's/^/        /' "${WORK}/e2e.out"; }
if [ "$(grep -c '^STUB-INSTALL' "${WORK}/stub.log")" = 2 ] \
    && grep -qx "STUB-INSTALL: birdnet-v24 (dir=${vol})" "${WORK}/stub.log" \
    && grep -qx "STUB-INSTALL: birdnet-v24-range-v2 (dir=${vol})" "${WORK}/stub.log"; then
    pass "the binary was asked for birdnet-v24 and birdnet-v24-range-v2, into the model directory"
else
    fail "binary calls: $(tr '\n' ';' <"${WORK}/stub.log")"
fi
[ "$(denv BIRDNET_MODEL)" = "${vol}/${V24_MODEL}" ] && [ "$(denv BIRDNET_LABELS)" = "${vol}/${V24_LABELS}" ] \
    && [ "$(denv BIRDNET_METADATA_MODEL)" = "${vol}/${V24_RANGE}" ] \
    && pass "the daemon starts with the V2.4 model, labels and range filter" || fail "daemon env: model '$(denv BIRDNET_MODEL)' filter '$(denv BIRDNET_METADATA_MODEL)'"
grep -q 'DAEMON-ARGS: --some-arg' "${WORK}/e2e.out" && pass "extra arguments are forwarded" || fail "arguments were not forwarded"
grep -c '\[birdnet\] Classifier:' "${WORK}/e2e.out" | grep -qx 1 && grep -q 'Classifier: BirdNET V2.4' "${WORK}/e2e.out" \
    && pass "one Classifier line, V2.4" || fail "Classifier lines: $(grep 'Classifier:' "${WORK}/e2e.out")"

run_entrypoint "${vol}"; rc=$?
if [ "${rc}" -eq 0 ] && [ ! -s "${WORK}/stub.log" ] && [ "$(denv BIRDNET_METADATA_MODEL)" = "${vol}/${V24_RANGE}" ]; then
    pass "the second start, on the same volume, installs nothing and still configures the filter"
else
    fail "second start: exit ${rc}; binary calls: $(tr '\n' ';' <"${WORK}/stub.log")"
fi

vol="${WORK}/e2e-v3"
mkdir -p "${vol}"; : >"${vol}/${V3_MODEL}"
run_entrypoint "${vol}" BIRDNET_SKIP_MODEL_DOWNLOAD=1; rc=$?
[ "${rc}" -eq 0 ] && [ "$(denv BIRDNET_MODEL)" = "${vol}/${V3_MODEL}" ] && [ ! -s "${WORK}/stub.log" ] \
    && pass "a volume holding the V3.0 model starts the daemon on it, without touching the binary's installer" || fail "exit ${rc}; model '$(denv BIRDNET_MODEL)'"
grep 'Classifier:' "${WORK}/e2e.out" | grep -q 'keeps running it' && pass "with the notice" || fail "no notice: $(grep 'Classifier:' "${WORK}/e2e.out")"

vol="${WORK}/e2e-v3-explicit"
run_entrypoint "${vol}" BIRDNET_CLASSIFIER=birdnet-v3 BIRDNET_SKIP_MODEL_DOWNLOAD=1; rc=$?
[ "${rc}" -eq 0 ] && [ "$(denv BIRDNET_MODEL)" = "${vol}/${V3_MODEL}" ] \
    && pass "BIRDNET_CLASSIFIER=birdnet-v3 on a new volume starts V3.0" || fail "exit ${rc}; model '$(denv BIRDNET_MODEL)'"

vol="${WORK}/e2e-blank"
run_entrypoint "${vol}" BIRDNET_CLASSIFIER=; rc=$?
[ "${rc}" -eq 0 ] && [ "$(denv BIRDNET_MODEL)" = "${vol}/${V24_MODEL}" ] \
    && pass "a blank BIRDNET_CLASSIFIER (what Compose's \${VAR:-} makes of an unset one) means unset" || fail "exit ${rc}; model '$(denv BIRDNET_MODEL)'"

# The daemon does not read BIRDNET_CLASSIFIER, and it reports every BIRDNET_*
# variable it does not read at each start.
vol="${WORK}/e2e-unset"
run_entrypoint "${vol}" BIRDNET_CLASSIFIER=birdnet-v24; rc=$?
if [ -z "$(denv BIRDNET_CLASSIFIER)" ] && [ "${rc}" -eq 0 ]; then
    pass "BIRDNET_CLASSIFIER is removed before the daemon starts (it would be reported as an unknown variable)"
else
    fail "the daemon would see BIRDNET_CLASSIFIER='$(denv BIRDNET_CLASSIFIER)'"
fi

if [ -s "${WORK}/stub.log" ] && ! grep -q 'STUB-SAW-BIRDNET_CLASSIFIER' "${WORK}/stub.log"; then
    pass "the binary's installs never see BIRDNET_CLASSIFIER (it would report it as an unknown variable)"
else
    fail "an --install-model run was handed BIRDNET_CLASSIFIER"
fi

vol="${WORK}/e2e-bad"
run_entrypoint "${vol}" BIRDNET_CLASSIFIER=nonsense; rc=$?
if [ "${rc}" -ne 0 ] && ! grep -q 'DAEMON-ARGS' "${WORK}/e2e.out" && grep -q 'birdnet-v24' "${WORK}/e2e.out" && [ ! -s "${WORK}/stub.log" ]; then
    pass "an unknown value stops the container before the daemon, naming the valid ones"
else
    fail "exit ${rc}; the daemon $(grep -q DAEMON-ARGS "${WORK}/e2e.out" && echo started || echo did not start)"
fi

vol="${WORK}/e2e-install-fails"
run_entrypoint "${vol}" STUB_FAIL=birdnet-v24; rc=$?
if [ "${rc}" -ne 0 ] && ! grep -q 'DAEMON-ARGS' "${WORK}/e2e.out" && grep -q 'BIRDNET_CLASSIFIER=birdnet-v3' "${WORK}/e2e.out"; then
    pass "a V2.4 that cannot be installed stops the container (the daemon would detect nothing) and offers birdnet-v3"
else
    fail "exit ${rc}; the daemon $(grep -q DAEMON-ARGS "${WORK}/e2e.out" && echo started || echo did not start)"
    sed 's/^/        /' "${WORK}/e2e.out" | tail -6
fi

vol="${WORK}/e2e-range-fails"
run_entrypoint "${vol}" STUB_FAIL=birdnet-v24-range-v2; rc=$?
if [ "${rc}" -eq 0 ] && [ -z "$(denv BIRDNET_METADATA_MODEL)" ] && [ "$(denv BIRDNET_MODEL)" = "${vol}/${V24_MODEL}" ] \
    && grep -q 'species occurrence filtering is OFF' "${WORK}/e2e.out"; then
    pass "a range filter that cannot be installed leaves a running container with filtering off, said so"
else
    fail "exit ${rc}; filter '$(denv BIRDNET_METADATA_MODEL)'"
fi

echo
if [ "${FAILED}" -ne 0 ]; then
    echo "classifier-docker.sh: FAILED"
    exit 1
fi
echo "classifier-docker.sh: all pass"
