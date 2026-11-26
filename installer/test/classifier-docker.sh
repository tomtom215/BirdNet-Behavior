#!/usr/bin/env bash
# installer/test/classifier-docker.sh — which classifier does the CONTAINER run,
# and how does it get its models?
#
# The same decision as the bare-metal installer, made by docker/entrypoint.sh:
# a new data volume gets BirdNET V2.4 and its range filter; the BirdNET+ V3.0
# preview is `BIRDNET_CLASSIFIER=birdnet-v3`; and a volume that already holds the
# V3.0 model and has no BIRDNET_CLASSIFIER set keeps running V3.0, with one log
# line saying so and how to change it — because an image upgrade must never swap
# the network under a running station's history.
#
# Every model is installed by the binary (`birdnet-behavior --install-model`,
# MDL-09), which owns the digests, the origins, the retries and the labels file
# V2.4 carries; the entrypoint has none of that. So the binary is stubbed here
# (BNB_BINARY, the same hook BNB_STRIP_LIB gives the blank-environment
# scrubber) by one that plays both parts the real one plays: with
# --install-model it does what the real one does with the files — writes them
# under the real names, adopts a file an earlier image left under its old V3.0
# name (a hard link), refuses a --from-file that is not the pinned body — and
# logs what it was asked and what it had to write; with anything else it is the
# daemon and prints what it was started with. `curl` on PATH is a stub that
# logs, so a download by the entrypoint itself is a failure here.
#
# This runs the WHOLE entrypoint, byte for byte, once per scenario. What it
# cannot show: that the image builds, that the real binary reaches its origins
# from inside a container, that Compose passes the variables through.
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

# The file names, as the contract says.
V24_MODEL="birdnet-v24.onnx"
V24_LABELS="birdnet-v24_labels.txt"
V24_RANGE="birdnet-v24-range-v2.onnx"
V3_MODEL="birdnet-v3.onnx"
V3_LABELS="birdnet-v3_labels.csv"
GEO_MODEL="birdnet-v3-geomodel.onnx"
GEO_LABELS="birdnet-v3-geomodel_labels.txt"
OLD_V3_MODEL="BirdNET+_V3.0-preview3_Global_11K_FP32.onnx"
OLD_V3_LABELS="BirdNET+_V3.0-preview3_Global_11K_Labels.csv"
OLD_GEO_MODEL="BirdNET+_Geomodel_V3.0.2_Global_12K_FP32.onnx"
OLD_GEO_LABELS="BirdNET+_Geomodel_V3.0.2_Global_12K_Labels.txt"

STUB="${WORK}/binary-stub"
cat >"${STUB}" <<'SH'
#!/bin/sh
id=""
from=""
next=""
for a in "$@"; do
    case "${next}" in
        id) id="${a}"; next=""; continue ;;
        from) from="${a}"; next=""; continue ;;
    esac
    [ "${a}" = "--install-model" ] && next=id
    [ "${a}" = "--from-file" ] && next=from
done
if [ -z "${id}" ]; then
    echo "DAEMON-ARGS: $*"
    env | sort | grep -E '^BIRDNET_' | sed 's/^/DAEMON-ENV: /'
    exit 0
fi
dir="${BIRDNET_MODEL_DIR}"
# The binary reports any BIRDNET_* variable it does not read.
[ -n "${BIRDNET_CLASSIFIER:-}" ] && echo "STUB-SAW-BIRDNET_CLASSIFIER" >>"${STUB_LOG}"
[ -n "${BIRDNET_LOCAL_MODEL:-}" ] && echo "STUB-SAW-BIRDNET_LOCAL_MODEL" >>"${STUB_LOG}"
if [ -n "${from}" ]; then
    echo "STUB-INSTALL: ${id} (dir=${dir}) from=${from}" >>"${STUB_LOG}"
else
    echo "STUB-INSTALL: ${id} (dir=${dir})" >>"${STUB_LOG}"
fi
echo "STUB-WAITS: ${id} ${BIRDNET_MODEL_RETRY_WAITS-<unset>}" >>"${STUB_LOG}"
case "${id}" in
    birdnet-v24) model=birdnet-v24.onnx; labels=birdnet-v24_labels.txt; old=""; old_labels="" ;;
    birdnet-v24-range-v2) model=birdnet-v24-range-v2.onnx; labels=""; old=""; old_labels="" ;;
    birdnet-v3) model=birdnet-v3.onnx; labels=birdnet-v3_labels.csv
        old="BirdNET+_V3.0-preview3_Global_11K_FP32.onnx"; old_labels="BirdNET+_V3.0-preview3_Global_11K_Labels.csv" ;;
    birdnet-v3-geomodel) model=birdnet-v3-geomodel.onnx; labels=birdnet-v3-geomodel_labels.txt
        old="BirdNET+_Geomodel_V3.0.2_Global_12K_FP32.onnx"; old_labels="BirdNET+_Geomodel_V3.0.2_Global_12K_Labels.txt" ;;
    *) echo "Install failed: no model called ${id}" >&2; exit 1 ;;
esac
# The "pinned" body of every file is "pinned <name>".
place() { # NAME OLD
    [ "$(cat "${dir}/$1" 2>/dev/null)" = "pinned $1" ] && return 0
    if [ -n "$2" ] && [ "$(cat "${dir}/$2" 2>/dev/null)" = "pinned $1" ]; then
        rm -f "${dir}/$1"; ln "${dir}/$2" "${dir}/$1"; echo "STUB-ADOPTED: $2" >>"${STUB_LOG}"; return 0
    fi
    [ "${STUB_FAIL:-}" = "${id}" ] && { echo "Install failed: no origin served the verified file" >&2; return 1; }
    echo "STUB-WROTE: $1" >>"${STUB_LOG}"
    echo "pinned $1" >"${dir}/$1"
}
if [ -n "${from}" ]; then
    [ "$(cat "${from}")" = "pinned ${model}" ] || { echo "Install failed: sha256 mismatch" >&2; exit 1; }
    echo "pinned ${model}" >"${dir}/${model}"
else
    place "${model}" "${old}" || exit 1
fi
[ -z "${labels}" ] || place "${labels}" "${old_labels}" || exit 1
exit 0
SH
chmod +x "${STUB}"
mkdir -p "${WORK}/bin"
printf '#!/bin/sh\necho "CURL $*" >>"${STUB_LOG}"\nexit 1\n' >"${WORK}/bin/curl"
chmod +x "${WORK}/bin/curl"

# Seed VOL with files whose content is the stub's "pinned" body for NAME, or,
# for OLD:NEW, NEW's body under the old name OLD.
seed() {
    local vol="$1" f; shift
    mkdir -p "${vol}"
    for f in "$@"; do
        case "${f}" in
            *:*) echo "pinned ${f#*:}" >"${vol}/${f%%:*}" ;;
            *) echo "pinned ${f}" >"${vol}/${f}" ;;
        esac
    done
}
old_v3_volume() { # the four V3.0 files as an earlier image left them
    seed "$1" "${OLD_V3_MODEL}:${V3_MODEL}" "${OLD_V3_LABELS}:${V3_LABELS}" \
        "${OLD_GEO_MODEL}:${GEO_MODEL}" "${OLD_GEO_LABELS}:${GEO_LABELS}"
}

run_entrypoint() { # volume dir, then extra env as NAME=VALUE...
    local vol="$1"; shift
    mkdir -p "${vol}"
    : >"${WORK}/stub.log"
    env -i PATH="${WORK}/bin:${PATH}" HOME="${WORK}" TZ=UTC STUB_LOG="${WORK}/stub.log" \
        BNB_STRIP_LIB="${REPO_ROOT}/docker/strip-blank-env.sh" BNB_BINARY="${STUB}" \
        BIRDNET_MODEL_DIR="${vol}" BIRDNET_ALSA_DEVICE=hw:0 "$@" \
        sh "${ENTRYPOINT}" --some-arg >"${WORK}/e2e.out" 2>&1
}
denv() { sed -n "s/^DAEMON-ENV: $1=//p" "${WORK}/e2e.out"; }
asked() { sed -n 's/^STUB-INSTALL: \([^ ]*\) .*/\1/p' "${WORK}/stub.log" | tr '\n' ' '; }
wrote() { grep -c '^STUB-WROTE' "${WORK}/stub.log" || true; }
classifier_lines() { grep -c '\[birdnet\] Classifier:' "${WORK}/e2e.out" || true; }
started() { grep -q 'DAEMON-ARGS: --some-arg' "${WORK}/e2e.out"; }
no_downloads_of_its_own() {
    if grep -q '^CURL' "${WORK}/stub.log"; then fail "$1: the entrypoint downloaded something itself: $(grep '^CURL' "${WORK}/stub.log" | head -2)"
    else pass "$1: the entrypoint downloaded nothing itself"; fi
}
# The "never mixed" rule, read off what the daemon is started with.
assert_unmixed() { # $1 label
    local m f
    m="$(denv BIRDNET_MODEL)"; f="$(denv BIRDNET_METADATA_MODEL)"
    case "${m##*/}" in
        "${V24_MODEL}")
            if [ -z "${f}" ] || [ "${f##*/}" = "${V24_RANGE}" ]; then pass "$1: V2.4 is never given the geomodel"
            else fail "$1: V2.4 was given '${f##*/}'"; fi
            [ -z "$(denv BIRDNET_METADATA_LABELS)" ] || fail "$1: V2.4 was given a metadata labels file"
            ;;
        "${V3_MODEL}")
            if [ -z "${f}" ] || [ "${f##*/}" = "${GEO_MODEL}" ]; then pass "$1: V3.0 is never given the range filter"
            else fail "$1: V3.0 was given '${f##*/}'"; fi
            ;;
    esac
}

echo "=== 1. a new volume runs BirdNET V2.4 and its range filter, both installed by the binary ==="
vol="${WORK}/new"
run_entrypoint "${vol}"; rc=$?
[ "${rc}" -eq 0 ] && started && pass "the entrypoint ran on to the daemon" || { fail "exit ${rc}"; sed 's/^/        /' "${WORK}/e2e.out"; }
[ "$(asked)" = "birdnet-v24 birdnet-v24-range-v2 " ] && grep -qx "STUB-INSTALL: birdnet-v24 (dir=${vol})" "${WORK}/stub.log" \
    && pass "the binary was asked for birdnet-v24 and birdnet-v24-range-v2, into the model directory" \
    || fail "binary calls: $(asked)"
[ "$(denv BIRDNET_MODEL)" = "${vol}/${V24_MODEL}" ] && [ "$(denv BIRDNET_LABELS)" = "${vol}/${V24_LABELS}" ] \
    && [ "$(denv BIRDNET_METADATA_MODEL)" = "${vol}/${V24_RANGE}" ] && [ -z "$(denv BIRDNET_METADATA_LABELS)" ] \
    && pass "the daemon starts with the V2.4 model, labels and range filter, and no filter labels" \
    || fail "daemon env: model '$(denv BIRDNET_MODEL)' filter '$(denv BIRDNET_METADATA_MODEL)'"
[ "$(classifier_lines)" = 1 ] && grep -q 'Classifier: BirdNET V2.4' "${WORK}/e2e.out" \
    && grep 'Classifier:' "${WORK}/e2e.out" | grep -q 'BIRDNET_CLASSIFIER=birdnet-v3' \
    && pass "one Classifier line, V2.4, saying how to choose the other" || fail "Classifier lines: $(grep 'Classifier:' "${WORK}/e2e.out")"
grep -q 'DAEMON-ARGS: --some-arg' "${WORK}/e2e.out" && pass "extra arguments are forwarded" || fail "arguments were not forwarded"
no_downloads_of_its_own "new volume"
assert_unmixed "new volume"

echo "=== 1b. the next start re-checks the files through the binary and downloads nothing ==="
run_entrypoint "${vol}"; rc=$?
if [ "${rc}" -eq 0 ] && [ "$(asked)" = "birdnet-v24 birdnet-v24-range-v2 " ] && [ "$(wrote)" = 0 ] \
    && [ "$(denv BIRDNET_METADATA_MODEL)" = "${vol}/${V24_RANGE}" ]; then
    pass "every start hands the files to the binary's check (a cached file is not trusted for being there), nothing is fetched, and the filter is on"
else
    fail "second start: exit ${rc}; calls: $(tr '\n' ';' <"${WORK}/stub.log")"
fi
echo "=== 1c. counterpart: a missing or damaged file IS fetched, and only that one ==="
vol="${WORK}/missing-model"
seed "${vol}" "${V24_LABELS}" "${V24_RANGE}"
run_entrypoint "${vol}"
[ "$(wrote)" = 1 ] && grep -qx "STUB-WROTE: ${V24_MODEL}" "${WORK}/stub.log" \
    && pass "a volume without the model gets the model, and keeps the rest" || fail "calls: $(tr '\n' ';' <"${WORK}/stub.log")"
vol="${WORK}/damaged-model"
seed "${vol}" "${V24_LABELS}" "${V24_RANGE}"
echo "truncated" >"${vol}/${V24_MODEL}"
run_entrypoint "${vol}"
grep -qx "STUB-WROTE: ${V24_MODEL}" "${WORK}/stub.log" && pass "a model with the wrong bytes is replaced" \
    || fail "a damaged model was trusted: $(tr '\n' ';' <"${WORK}/stub.log")"

echo "=== 1d. a range filter put on the volume since the last start is switched on by this one ==="
vol="${WORK}/filter-later"
seed "${vol}" "${V24_MODEL}" "${V24_LABELS}"
run_entrypoint "${vol}" STUB_FAIL=birdnet-v24-range-v2
[ -z "$(denv BIRDNET_METADATA_MODEL)" ] && pass "precondition: without the file, no filter" || fail "precondition: a filter was claimed"
seed "${vol}" "${V24_RANGE}"
run_entrypoint "${vol}" STUB_FAIL=birdnet-v24-range-v2
[ "$(denv BIRDNET_METADATA_MODEL)" = "${vol}/${V24_RANGE}" ] && pass "with it verified on the volume, the daemon is pointed at it" \
    || fail "the filter on the volume was not exported: '$(denv BIRDNET_METADATA_MODEL)'"

echo "=== 2. a volume an earlier image left with the V3.0 files keeps running V3.0, and keeps the files ==="
vol="${WORK}/old-v3"
old_v3_volume "${vol}"
inode="$(stat -c %i "${vol}/${OLD_V3_MODEL}")"
run_entrypoint "${vol}"; rc=$?
[ "${rc}" -eq 0 ] && started && pass "the entrypoint ran on to the daemon" || { fail "exit ${rc}"; sed 's/^/        /' "${WORK}/e2e.out" | tail -6; }
[ "$(asked)" = "birdnet-v3 birdnet-v3-geomodel " ] && pass "the binary was asked for birdnet-v3 and its geomodel" || fail "calls: $(asked)"
[ "$(wrote)" = 0 ] && [ "$(grep -c '^STUB-ADOPTED' "${WORK}/stub.log")" = 4 ] \
    && pass "nothing was downloaded: the four files were adopted where they were" || fail "calls: $(tr '\n' ';' <"${WORK}/stub.log")"
[ "$(stat -c %i "${vol}/${V3_MODEL}" 2>/dev/null)" = "${inode}" ] && pass "the model is the same file (no 541 MB copy)" || fail "the model was copied or replaced"
[ "$(denv BIRDNET_MODEL)" = "${vol}/${V3_MODEL}" ] && [ "$(denv BIRDNET_LABELS)" = "${vol}/${V3_LABELS}" ] \
    && [ "$(denv BIRDNET_METADATA_MODEL)" = "${vol}/${GEO_MODEL}" ] && [ "$(denv BIRDNET_METADATA_LABELS)" = "${vol}/${GEO_LABELS}" ] \
    && pass "the daemon starts on V3.0 and the geomodel, under the binary's names" \
    || fail "daemon env: model '$(denv BIRDNET_MODEL)' filter '$(denv BIRDNET_METADATA_MODEL)'"
left=""
for f in "${vol}"/BirdNET+_*; do [ -e "${f}" ] && left="${left} ${f##*/}"; done
if [ -n "${left}" ]; then fail "old names left:${left}"; else pass "the old names are gone"; fi
if [ "$(classifier_lines)" = 1 ] && grep 'Classifier:' "${WORK}/e2e.out" | grep -q 'keeps running it' \
    && grep 'Classifier:' "${WORK}/e2e.out" | grep -q 'BIRDNET_CLASSIFIER=birdnet-v24'; then
    pass "exactly one log line says it kept V3.0 and how to move to V2.4"
else
    fail "the notice: $(grep 'Classifier:' "${WORK}/e2e.out")"
fi
assert_unmixed "V3.0 volume"
no_downloads_of_its_own "V3.0 volume"

echo "=== 2b. a BIRDNET_MODEL pointed at the old V3.0 name follows the file to its new one ==="
vol="${WORK}/old-v3-named"
old_v3_volume "${vol}"
run_entrypoint "${vol}" BIRDNET_MODEL="${vol}/${OLD_V3_MODEL}" BIRDNET_METADATA_MODEL="${vol}/${OLD_GEO_MODEL}"
[ "$(denv BIRDNET_MODEL)" = "${vol}/${V3_MODEL}" ] && [ "$(denv BIRDNET_METADATA_MODEL)" = "${vol}/${GEO_MODEL}" ] \
    && [ -e "${vol}/${V3_MODEL}" ] && [ ! -e "${vol}/${OLD_V3_MODEL}" ] \
    && pass "BIRDNET_MODEL and BIRDNET_METADATA_MODEL name the adopted files, and the old names are gone" \
    || fail "model '$(denv BIRDNET_MODEL)' filter '$(denv BIRDNET_METADATA_MODEL)'; $(ls "${vol}" | tr '\n' ' ')"

echo "=== 3. BIRDNET_CLASSIFIER=birdnet-v3 on a new volume is V3.0 and its geomodel ==="
vol="${WORK}/v3-new"
run_entrypoint "${vol}" BIRDNET_CLASSIFIER=birdnet-v3
[ "$(denv BIRDNET_MODEL)" = "${vol}/${V3_MODEL}" ] && [ "$(denv BIRDNET_METADATA_MODEL)" = "${vol}/${GEO_MODEL}" ] \
    && [ "$(asked)" = "birdnet-v3 birdnet-v3-geomodel " ] \
    && pass "the binary installed the V3.0 model and the geomodel" || fail "model '$(denv BIRDNET_MODEL)' calls '$(asked)'"
grep 'Classifier:' "${WORK}/e2e.out" | grep -q 'BIRDNET_CLASSIFIER=birdnet-v3' && pass "the log credits BIRDNET_CLASSIFIER" || fail "the log does not credit BIRDNET_CLASSIFIER"
no_downloads_of_its_own "explicit V3.0"
assert_unmixed "explicit V3.0"

echo "=== 4. BIRDNET_CLASSIFIER=birdnet-v24 on a V3.0 volume moves it, because someone asked ==="
vol="${WORK}/v3-to-v24"
old_v3_volume "${vol}"
run_entrypoint "${vol}" BIRDNET_CLASSIFIER=birdnet-v24
[ "$(denv BIRDNET_MODEL)" = "${vol}/${V24_MODEL}" ] && [ "$(denv BIRDNET_METADATA_MODEL)" = "${vol}/${V24_RANGE}" ] \
    && pass "the V2.4 model and the range filter" || fail "model '$(denv BIRDNET_MODEL)' filter '$(denv BIRDNET_METADATA_MODEL)'"
assert_unmixed "explicit V2.4 on a V3.0 volume"

echo "=== 5. a value that is neither id stops the container, naming the two that are ==="
vol="${WORK}/bad"
run_entrypoint "${vol}" BIRDNET_CLASSIFIER=birdnet-v2.4; rc=$?
if [ "${rc}" -ne 0 ] && ! started && grep -q 'birdnet-v24' "${WORK}/e2e.out" && grep -q 'birdnet-v3' "${WORK}/e2e.out" && [ ! -s "${WORK}/stub.log" ]; then
    pass "exit ${rc}, both valid ids named, nothing installed"
else
    fail "exit ${rc}; output: $(tail -3 "${WORK}/e2e.out")"
fi
vol="${WORK}/blank"
run_entrypoint "${vol}" BIRDNET_CLASSIFIER=; rc=$?
[ "${rc}" -eq 0 ] && [ "$(denv BIRDNET_MODEL)" = "${vol}/${V24_MODEL}" ] \
    && pass "a blank BIRDNET_CLASSIFIER (what Compose's \${VAR:-} makes of an unset one) means unset" || fail "exit ${rc}; model '$(denv BIRDNET_MODEL)'"
vol="${WORK}/unset"
run_entrypoint "${vol}" BIRDNET_CLASSIFIER=birdnet-v24; rc=$?
[ -z "$(denv BIRDNET_CLASSIFIER)" ] && [ "${rc}" -eq 0 ] \
    && pass "BIRDNET_CLASSIFIER is removed before the daemon starts (it would be reported as an unknown variable)" \
    || fail "the daemon would see BIRDNET_CLASSIFIER='$(denv BIRDNET_CLASSIFIER)'"
! grep -q 'STUB-SAW-BIRDNET_CLASSIFIER' "${WORK}/stub.log" && [ -s "${WORK}/stub.log" ] \
    && pass "and the binary's installs never see it" || fail "an --install-model run was handed BIRDNET_CLASSIFIER"

echo "=== 6. a model of the operator's own keeps the V3.0 defaults it always had ==="
vol="${WORK}/own"
run_entrypoint "${vol}" BIRDNET_MODEL=/data/mine/my-own.onnx
if [ "$(denv BIRDNET_MODEL)" = "/data/mine/my-own.onnx" ] && [ "$(asked)" = "birdnet-v3-geomodel " ] \
    && [ "$(denv BIRDNET_METADATA_MODEL)" = "${vol}/${GEO_MODEL}" ]; then
    pass "their model is untouched and not installed over; the geomodel defaults as before"
else
    fail "own-model handling: model '$(denv BIRDNET_MODEL)', calls '$(asked)', filter '$(denv BIRDNET_METADATA_MODEL)'"
fi
vol="${WORK}/own-v24"
run_entrypoint "${vol}" BIRDNET_MODEL="${vol}/${V24_MODEL}"
grep 'Classifier:' "${WORK}/e2e.out" | grep -q 'BIRDNET_MODEL is the V2.4 file' \
    && pass "BIRDNET_MODEL naming the V2.4 file is V2.4" || fail "selected: $(grep 'Classifier:' "${WORK}/e2e.out")"

echo "=== 7. an operator-set BIRDNET_METADATA_MODEL is theirs ==="
vol="${WORK}/own-filter"
run_entrypoint "${vol}" BIRDNET_METADATA_MODEL=/data/mine/filter.onnx
[ "$(denv BIRDNET_METADATA_MODEL)" = "/data/mine/filter.onnx" ] && [ "$(asked)" = "birdnet-v24 " ] \
    && pass "the range filter was not installed over it" || fail "filter '$(denv BIRDNET_METADATA_MODEL)', calls '$(asked)'"

echo "=== 8. BIRDNET_METADATA_LABELS beside the range filter is flagged, not silently paired ==="
vol="${WORK}/labels-beside-range"
run_entrypoint "${vol}" BIRDNET_METADATA_LABELS=/data/mine/geo-labels.txt
grep -q 'WARNING: BIRDNET_METADATA_LABELS is set, but the V2.4 range filter' "${WORK}/e2e.out" \
    && pass "a warning says the V2.4 range filter needs no labels file" || fail "no warning for a label file set beside the V2.4 range filter"

echo "=== 9. BIRDNET_SKIP_MODEL_DOWNLOAD=1 installs nothing and still says which classifier ==="
vol="${WORK}/skip"
run_entrypoint "${vol}" BIRDNET_SKIP_MODEL_DOWNLOAD=1
[ ! -s "${WORK}/stub.log" ] && [ "$(denv BIRDNET_MODEL)" = "${vol}/${V24_MODEL}" ] && [ -z "$(denv BIRDNET_METADATA_MODEL)" ] && [ "$(classifier_lines)" = 1 ] \
    && pass "no install, the V2.4 paths are set, no filter is claimed, one Classifier line" \
    || fail "skip handling: calls '$(asked)' model '$(denv BIRDNET_MODEL)' filter '$(denv BIRDNET_METADATA_MODEL)'"
vol="${WORK}/skip-old-v3"
seed "${vol}" "${OLD_V3_MODEL}:${V3_MODEL}"
run_entrypoint "${vol}" BIRDNET_SKIP_MODEL_DOWNLOAD=1
grep 'Classifier:' "${WORK}/e2e.out" | grep -q 'keeps running it' && [ ! -s "${WORK}/stub.log" ] \
    && pass "a volume holding the old V3.0 model is still recognised as V3.0" || fail "$(grep 'Classifier:' "${WORK}/e2e.out")"

echo "=== 10. the filter failing leaves a working container without it ==="
vol="${WORK}/range-fails"
run_entrypoint "${vol}" STUB_FAIL=birdnet-v24-range-v2; rc=$?
[ "${rc}" -eq 0 ] && started && [ -z "$(denv BIRDNET_METADATA_MODEL)" ] && [ "$(denv BIRDNET_MODEL)" = "${vol}/${V24_MODEL}" ] \
    && grep -q 'species occurrence filtering is OFF' "${WORK}/e2e.out" \
    && pass "the classifier is configured, no filter is claimed, and the log says filtering is off" \
    || fail "exit ${rc}; filter '$(denv BIRDNET_METADATA_MODEL)'"
vol="${WORK}/geo-fails"
run_entrypoint "${vol}" BIRDNET_CLASSIFIER=birdnet-v3 STUB_FAIL=birdnet-v3-geomodel
[ -z "$(denv BIRDNET_METADATA_MODEL)" ] && [ -z "$(denv BIRDNET_METADATA_LABELS)" ] && grep -q 'species occurrence filtering is OFF' "${WORK}/e2e.out" \
    && pass "a geomodel that cannot be installed: neither half is configured" || fail "filter '$(denv BIRDNET_METADATA_MODEL)' labels '$(denv BIRDNET_METADATA_LABELS)'"

echo "=== 11. a classifier that cannot be installed: the container starts anyway, and says so ==="
for id in birdnet-v24 birdnet-v3; do
    vol="${WORK}/fails-${id}"
    run_entrypoint "${vol}" BIRDNET_CLASSIFIER="${id}" STUB_FAIL="${id}" BIRDNET_MODEL_RETRY_WAITS="0 0"; rc=$?
    if [ "${rc}" -eq 0 ] && started && grep -q 'NOT DETECTING BIRDS' "${WORK}/e2e.out" \
        && grep -q 'BIRDNET_LOCAL_MODEL' "${WORK}/e2e.out" && grep -q 'Starting WITHOUT a classifier' "${WORK}/e2e.out"; then
        pass "${id}: the daemon (and the dashboard) start, and the log says nothing is being identified and how to fix it"
    else
        fail "${id}: exit ${rc}; $(tail -6 "${WORK}/e2e.out" | tr '\n' '|')"
    fi
    [ "$(grep -c "^STUB-INSTALL: ${id} " "${WORK}/stub.log")" = 1 ] && grep -qx "STUB-WAITS: ${id} 0 0" "${WORK}/stub.log" \
        && pass "${id}: asked once, with the retry schedule, which the binary carries out" \
        || fail "${id}: $(tr '\n' ';' <"${WORK}/stub.log")"
done
vol="${WORK}/default-waits"
run_entrypoint "${vol}"
grep -qx 'STUB-WAITS: birdnet-v24 <unset>' "${WORK}/stub.log" \
    && pass "unset, the schedule is the binary's own default (30 90)" || fail "$(grep WAITS "${WORK}/stub.log")"
! grep -q 'NOT DETECTING' "${WORK}/e2e.out" && pass "counterpart: an install that works gives no warning" || fail "a working install warned"

echo "=== 12. a local model file is handed to the binary's own check ==="
vol="${WORK}/local"
echo "pinned ${V24_MODEL}" >"${WORK}/local.onnx"
run_entrypoint "${vol}" BIRDNET_LOCAL_MODEL="${WORK}/local.onnx"; rc=$?
if [ "${rc}" -eq 0 ] && grep -qx "STUB-INSTALL: birdnet-v24 (dir=${vol}) from=${WORK}/local.onnx" "${WORK}/stub.log" \
    && [ -s "${vol}/${V24_MODEL}" ] && [ -z "$(denv BIRDNET_LOCAL_MODEL)" ] && ! grep -q 'STUB-SAW-BIRDNET_LOCAL_MODEL' "${WORK}/stub.log"; then
    pass "BIRDNET_LOCAL_MODEL is installed through --from-file, and passed to neither the binary's environment nor the daemon"
else
    fail "BIRDNET_LOCAL_MODEL: exit ${rc}; calls: $(tr '\n' ';' <"${WORK}/stub.log")"
fi
vol="${WORK}/local-wrong"
echo "something else" >"${WORK}/wrong.onnx"
run_entrypoint "${vol}" BIRDNET_LOCAL_MODEL="${WORK}/wrong.onnx"; rc=$?
[ "${rc}" -eq 0 ] && [ ! -e "${vol}/${V24_MODEL}" ] && grep -q 'was refused' "${WORK}/e2e.out" && started \
    && pass "a local file that does not match is refused, nothing is installed, and the container still starts" \
    || fail "a wrong BIRDNET_LOCAL_MODEL: exit ${rc}"

echo "=== 13. the old download mirrors are reported, not silently ignored ==="
vol="${WORK}/old-mirror"
run_entrypoint "${vol}" BIRDNET_MODEL_BASE=https://mirror.example/models
grep -q 'BIRDNET_MODEL_BASE is no longer read' "${WORK}/e2e.out" && grep -q 'BIRDNET_MODEL_MIRROR' "${WORK}/e2e.out" \
    && [ -z "$(denv BIRDNET_MODEL_BASE)" ] && pass "a warning names the variable that replaced it, and the daemon is not handed it" \
    || fail "no warning for BIRDNET_MODEL_BASE"

echo
if [ "${FAILED}" -ne 0 ]; then
    echo "classifier-docker.sh: FAILED"
    exit 1
fi
echo "classifier-docker.sh: all pass"
