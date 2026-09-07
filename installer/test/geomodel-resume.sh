#!/usr/bin/env bash
# installer/test/geomodel-resume.sh — a partial or half-present geomodel must be
# verified, not taken as installed because a file exists.
#
# ## What was wrong (ON-3)
#
# LC-2 taught `download_model` to ask `model_file_is_verified` — a checksum —
# instead of `[ -f ]`, because a partial download is a file. The geomodel half
# of the same module kept the presence guard:
#
#     if [ -f "${model_dest}" ] && [ -f "${labels_dest}" ]; then
#         GEOMODEL_INSTALLED=1
#
# and each download branch was gated on `[ ! -f ]` too. So a geomodel whose
# fetch dropped at 60 % was "already present" on every re-run and every
# `repair`, `GEOMODEL_INSTALLED=1` made 62-config-file.sh write the
# `METADATA_*` settings for it, and the daemon then refused the pair on every
# start — with the occurrence filter, the one thing that keeps a station's
# species list local, silently off. The classifier could not reach that state
# any more; the geomodel still could. Third instance of one shape.
#
# ## What this gate holds
#
#   1. a truncated geomodel is re-fetched, not skipped;
#   2. a verified pair is NOT re-downloaded, and is reported installed;
#   3. a truncated labels file is re-fetched and the good model left alone;
#   4. a pair with the labels missing does not count as installed without a
#      fetch — presence of one file is not presence of the pair;
#   5. an absent pair is fetched, which is the ordinary first install.
#
# (2) is the discrimination: a guard that always re-downloaded would satisfy
# (1), (3) and (4).
#
# Observed failing against the shipped presence-only guard: (1) and (3) fail
# with "the truncated geomodel was skipped — this is the defect" / "the
# truncated geomodel labels were skipped".
#
# Usage: installer/test/geomodel-resume.sh
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LIB="${HERE}/../lib"
FAILED=0

pass() { echo "  PASS  $*"; }
fail() { echo "  FAIL  $*"; FAILED=1; }

MODEL_BODY='the real geomodel bytes, all of them'
LABELS_BODY='Pica pica_Eurasian Magpie
Turdus merula_Eurasian Blackbird'
MODEL_SHA="$(printf '%s' "${MODEL_BODY}" | sha256sum | awk '{print $1}')"
LABELS_SHA="$(printf '%s' "${LABELS_BODY}" | sha256sum | awk '{print $1}')"

# Drive the shipping `download_geomodel` with the network helper stubbed, and
# report which files it asked for and whether it declared the pair installed.
#
#   run_download_geomodel <sandbox>
#
# Echoes the exit status; ${sandbox}/calls.log lists what was fetched and
# ${sandbox}/installed holds GEOMODEL_INSTALLED afterwards.
run_download_geomodel() {
    local sandbox="$1"
    (
        set -uo pipefail
        # The real functions, from the real module.
        # shellcheck disable=SC1090
        source <(sed -n '/^verify_model_sha256()/,/^}/p' "${LIB}/55-model.sh")
        # shellcheck disable=SC1090
        source <(sed -n '/^model_file_is_verified()/,/^}/p' "${LIB}/55-model.sh")
        # shellcheck disable=SC1090
        source <(sed -n '/^download_geomodel()/,/^}/p' "${LIB}/55-model.sh")

        info()       { echo "[INFO] $*"; }
        success()    { echo "[OK] $*"; }
        warn()       { echo "[WARN] $*"; }
        fatal()      { echo "[FATAL] $*"; exit 1; }
        loud_warn()  { echo "[LOUD] $*"; }
        chown()      { :; }
        geomodel_origins() { echo "github https://example.invalid/$1"; }

        # The stub network. Records the request and writes the *correct* bytes,
        # so a re-fetch always succeeds and the only thing under test is
        # whether the fetch happened at all.
        fetch_verified_model() {
            local dest="$1"
            echo "${dest##*/}" >>"${sandbox}/calls.log"
            case "${dest##*/}" in
                "${GEOMODEL_FILE}")        printf '%s' "${MODEL_BODY}"  >"${dest}" ;;
                "${GEOMODEL_LABELS_FILE}") printf '%s' "${LABELS_BODY}" >"${dest}" ;;
                *) return 1 ;;
            esac
            return 0
        }

        # install.sh globals (10-config.sh), read by the sourced body. Invisible
        # to shellcheck because `-x` does not follow a process substitution —
        # see model-resume.sh for the longer version of this note.
        # shellcheck disable=SC2034
        {
        MODEL_DIR="${sandbox}/models"
        GEOMODEL_FILE="geomodel.onnx"
        GEOMODEL_LABELS_FILE="geomodel_labels.txt"
        GEOMODEL_SHA256="${MODEL_SHA}"
        GEOMODEL_LABELS_SHA256="${LABELS_SHA}"
        GEOMODEL_VERSION="vtest"
        SERVICE_USER="birdnet"
        BIRDNET_SKIP_MODEL=0
        GEOMODEL_INSTALLED=0
        }

        # `install -d -o birdnet` needs root; this test does not run as root.
        install() { command install "${@/#-o birdnet/}" 2>/dev/null || command mkdir -p "${!#}"; }

        download_geomodel
        rc=$?
        echo "${GEOMODEL_INSTALLED}" >"${sandbox}/installed"
        exit "${rc}"
    ) >"${sandbox}/out.log" 2>&1
    echo $?
}

setup() {
    local sandbox="$1"
    mkdir -p "${sandbox}/models"
    : >"${sandbox}/calls.log"
}

echo "=== 1. a truncated geomodel is re-fetched, not skipped ==="
SB="$(mktemp -d)"; setup "${SB}"
printf 'PARTIAL-GARBAGE-NOT-THE-GEOMODEL' >"${SB}/models/geomodel.onnx"
printf '%s' "${LABELS_BODY}" >"${SB}/models/geomodel_labels.txt"
RC="$(run_download_geomodel "${SB}")"
if grep -q 'geomodel.onnx' "${SB}/calls.log"; then
    pass "the truncated geomodel was re-fetched"
else
    fail "the truncated geomodel was skipped — this is the defect"
    sed 's/^/        /' "${SB}/out.log" | head -5
fi
if [ "$(cat "${SB}/models/geomodel.onnx")" = "${MODEL_BODY}" ]; then
    pass "and the file on disk is now the real geomodel"
else
    fail "the file on disk is still the partial"
fi
if [ "$(cat "${SB}/installed")" = 1 ]; then
    pass "and the pair is reported installed once it verifies"
else
    fail "a verified pair was not reported installed"
fi
[ "${RC}" = 0 ] || fail "download_geomodel failed (exit ${RC})"
rm -rf "${SB}"

echo "=== 2. counterpart: a verified pair is NOT re-downloaded ==="
SB="$(mktemp -d)"; setup "${SB}"
printf '%s' "${MODEL_BODY}"  >"${SB}/models/geomodel.onnx"
printf '%s' "${LABELS_BODY}" >"${SB}/models/geomodel_labels.txt"
RC="$(run_download_geomodel "${SB}")"
if [ -s "${SB}/calls.log" ]; then
    fail "a verified geomodel was re-downloaded; every re-run would cost the transfer"
    sed 's/^/        /' "${SB}/calls.log"
else
    pass "nothing was fetched"
fi
if [ "$(cat "${SB}/installed")" = 1 ]; then
    pass "and the pair is reported installed"
else
    fail "a verified pair was not reported installed"
fi
if grep -q 'verified' "${SB}/out.log"; then
    pass "and it said the files were verified, not merely present"
else
    fail "the skip message does not distinguish verified from present"
    sed 's/^/        /' "${SB}/out.log" | head -3
fi
[ "${RC}" = 0 ] || fail "download_geomodel failed (exit ${RC})"
rm -rf "${SB}"

echo "=== 3. a truncated labels file is re-fetched too ==="
SB="$(mktemp -d)"; setup "${SB}"
printf '%s' "${MODEL_BODY}" >"${SB}/models/geomodel.onnx"
printf 'trunc' >"${SB}/models/geomodel_labels.txt"
RC="$(run_download_geomodel "${SB}")"
if grep -q 'geomodel_labels.txt' "${SB}/calls.log"; then
    pass "the truncated geomodel labels were re-fetched"
else
    fail "the truncated geomodel labels were skipped"
fi
if grep -q 'geomodel.onnx' "${SB}/calls.log"; then
    fail "the good geomodel was re-downloaded as well; only the bad file should move"
else
    pass "and the good geomodel was left alone"
fi
[ "${RC}" = 0 ] || fail "download_geomodel failed (exit ${RC})"
rm -rf "${SB}"

echo "=== 4. one file present is not the pair installed ==="
SB="$(mktemp -d)"; setup "${SB}"
printf '%s' "${MODEL_BODY}" >"${SB}/models/geomodel.onnx"
RC="$(run_download_geomodel "${SB}")"
if grep -q 'geomodel_labels.txt' "${SB}/calls.log"; then
    pass "the missing labels were fetched"
else
    fail "a model without its labels was accepted without fetching them"
fi
[ "$(cat "${SB}/installed")" = 1 ] || fail "the completed pair was not reported installed"
[ "${RC}" = 0 ] || fail "download_geomodel failed (exit ${RC})"
rm -rf "${SB}"

echo "=== 5. an absent pair is fetched, which is the ordinary first install ==="
SB="$(mktemp -d)"; setup "${SB}"
RC="$(run_download_geomodel "${SB}")"
if grep -q 'geomodel.onnx' "${SB}/calls.log" && grep -q 'geomodel_labels.txt' "${SB}/calls.log"; then
    pass "both files were fetched"
else
    fail "a first install did not fetch both files"
    sed 's/^/        /' "${SB}/out.log" | head -5
fi
[ "$(cat "${SB}/installed")" = 1 ] || fail "the fetched pair was not reported installed"
[ "${RC}" = 0 ] || fail "download_geomodel failed (exit ${RC})"
rm -rf "${SB}"

if [ "${FAILED}" -eq 0 ]; then
    echo
    echo "geomodel-resume: all pass"
else
    echo
    echo "geomodel-resume: FAILURES"
fi
exit "${FAILED}"
