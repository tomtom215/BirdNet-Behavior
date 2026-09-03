#!/usr/bin/env bash
# installer/test/model-resume.sh — a partial model download must be resumed, not
# mistaken for a finished one.
#
# ## What was wrong
#
# Every guard around the model asked `[ -f "${dest}" ]`. A partial download is a
# file. So:
#
#   1. a 541 MB fetch drops at 60 % and `fetch_verified_model` fails;
#   2. the failure path deliberately KEEPS the partial and prints "Re-run this
#      installer to resume from where it stopped";
#   3. the operator re-runs, and the presence guard skips the fetch entirely;
#   4. the installer prints "Model already downloaded — skipping" and
#      "Validation passed";
#   5. `install.sh repair` — the documented wizard for a broken install — says
#      "Model present — skipping download" and computes no checksum;
#   6. `--doctor` passes it, because `src/doctor/model.rs` accepts any file over
#      one megabyte;
#   7. the daemon logs "failed to start detection daemon", returns `None`, and
#      `app.rs` carries on and serves the web UI;
#   8. `/api/v2/health` answers `200 "healthy"`, because its status is SQLite's
#      and nothing else.
#
# The operator seals the box, drives it forty kilometres out, and gets a green
# dashboard that never records a bird. The only signal is the detection deadman,
# and only if a notifier is configured.
#
# ## What this gate holds
#
#   1. a truncated model is re-fetched, not skipped;
#   2. a verified model is skipped, so a re-run does not re-download 541 MB;
#   3. a truncated *labels* file is re-fetched too;
#   4. neither guard is satisfied by presence alone.
#
# (2) is the discrimination: a guard that always re-downloaded would satisfy
# (1) and (3) and cost every operator a 541 MB transfer on every re-run.
#
# Observed failing against the shipped presence-only guards: (1) and (3) fail
# with "the truncated model was skipped — this is the defect". (2)'s
# message assertion passes either way, because that revert changed the three
# `if` conditions and not the wording of the skip line; it is kept as a check on
# the wording, not as part of the demonstration.
#
# Usage: installer/test/model-resume.sh
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LIB="${HERE}/../lib"
FAILED=0

pass() { echo "  PASS  $*"; }
fail() { echo "  FAIL  $*"; FAILED=1; }

MODEL_BODY='the real model bytes, all of them'
LABELS_BODY='sci,common
Pica pica,Eurasian Magpie'
MODEL_SHA="$(printf '%s' "${MODEL_BODY}" | sha256sum | awk '{print $1}')"
LABELS_SHA="$(printf '%s' "${LABELS_BODY}" | sha256sum | awk '{print $1}')"

# Drive the shipping `download_model` with the network helper stubbed, and
# report which files it asked for.
#
#   run_download_model <sandbox>
#
# Echoes the exit status; ${sandbox}/calls.log lists what was fetched.
run_download_model() {
    local sandbox="$1"
    (
        set -uo pipefail
        # The real functions, from the real modules.
        # shellcheck disable=SC1090
        source <(sed -n '/^verify_model_sha256()/,/^}/p' "${LIB}/55-model.sh")
        # shellcheck disable=SC1090
        source <(sed -n '/^model_file_is_verified()/,/^}/p' "${LIB}/55-model.sh")
        # shellcheck disable=SC1090
        source <(sed -n '/^download_model()/,/^}/p' "${LIB}/55-model.sh")

        info()       { echo "[INFO] $*"; }
        success()    { echo "[OK] $*"; }
        warn()       { echo "[WARN] $*"; }
        fatal()      { echo "[FATAL] $*"; exit 1; }
        loud_warn()  { echo "[LOUD] $*"; }
        chown()      { :; }
        classifier_origins() { echo "github https://example.invalid/$1"; }

        # The stub network. Records the request and writes the *correct* bytes,
        # so a re-fetch always succeeds and the only thing under test is
        # whether the fetch happened at all.
        fetch_verified_model() {
            local dest="$1"
            echo "${dest##*/}" >>"${sandbox}/calls.log"
            case "${dest##*/}" in
                "${MODEL_FILE}")  printf '%s' "${MODEL_BODY}"  >"${dest}" ;;
                "${LABELS_FILE}") printf '%s' "${LABELS_BODY}" >"${dest}" ;;
                *) return 1 ;;
            esac
            return 0
        }

        # install.sh globals (10-config.sh), read by the `download_model` body
        # sourced above — 55-model.sh reads MODEL_DIR at 145, MODEL_SHA256 at
        # 152, LABELS_SHA256 at 153 and BIRDNET_SKIP_MODEL at 163.
        #
        # Every one is invisible to shellcheck, and the reason is worth stating
        # once rather than five times: the function bodies arrive through
        # `source <(sed -n ...)`, and `-x` does not follow a process
        # substitution, so a global the sourced code reads looks unused here.
        # SC2034 is therefore blanket-disabled for this block. Two of these
        # assignments were reported by CI and three more were only found by
        # running shellcheck locally, because the CI log was read from its
        # tail.
        #
        # BIRDNET_SKIP_MODEL is 0 deliberately: this test must exercise the
        # download path, not the air-gapped skip.
        # shellcheck disable=SC2034
        {
        MODEL_DIR="${sandbox}/models"
        MODEL_FILE="BirdNET_V3.onnx"
        LABELS_FILE="labels.csv"
        MODEL_SHA256="${MODEL_SHA}"
        LABELS_SHA256="${LABELS_SHA}"
        SERVICE_USER="birdnet"
        MODEL_RELEASE_TAG="models-test"
        BIRDNET_SKIP_MODEL=0
        }

        # `install -d -o birdnet` needs root; this test does not run as root.
        install() { command install "${@/#-o birdnet/}" 2>/dev/null || command mkdir -p "${!#}"; }

        download_model
    ) >"${sandbox}/out.log" 2>&1
    echo $?
}

setup() {
    local sandbox="$1"
    mkdir -p "${sandbox}/models"
    : >"${sandbox}/calls.log"
}

echo "=== 1. a truncated model is re-fetched, not skipped ==="
SB="$(mktemp -d)"; setup "${SB}"
printf 'PARTIAL-GARBAGE-NOT-THE-MODEL' >"${SB}/models/BirdNET_V3.onnx"
printf '%s' "${LABELS_BODY}" >"${SB}/models/labels.csv"
RC="$(run_download_model "${SB}")"
if grep -q 'BirdNET_V3.onnx' "${SB}/calls.log"; then
    pass "the truncated model was re-fetched"
else
    fail "the truncated model was skipped — this is the defect"
    sed 's/^/        /' "${SB}/out.log" | head -5
fi
if [ "$(cat "${SB}/models/BirdNET_V3.onnx")" = "${MODEL_BODY}" ]; then
    pass "and the file on disk is now the real model"
else
    fail "the file on disk is still the partial"
fi
[ "${RC}" = 0 ] || fail "download_model failed (exit ${RC})"
rm -rf "${SB}"

echo "=== 2. counterpart: a verified model is NOT re-downloaded ==="
SB="$(mktemp -d)"; setup "${SB}"
printf '%s' "${MODEL_BODY}"  >"${SB}/models/BirdNET_V3.onnx"
printf '%s' "${LABELS_BODY}" >"${SB}/models/labels.csv"
RC="$(run_download_model "${SB}")"
if [ -s "${SB}/calls.log" ]; then
    fail "a verified model was re-downloaded; every re-run would cost 541 MB"
    sed 's/^/        /' "${SB}/calls.log"
else
    pass "nothing was fetched"
fi
if grep -q 'verified' "${SB}/out.log"; then
    pass "and it said the files were verified, not merely present"
else
    fail "the skip message does not distinguish verified from present"
    sed 's/^/        /' "${SB}/out.log" | head -3
fi
[ "${RC}" = 0 ] || fail "download_model failed (exit ${RC})"
rm -rf "${SB}"

echo "=== 3. a truncated labels file is re-fetched too ==="
SB="$(mktemp -d)"; setup "${SB}"
printf '%s' "${MODEL_BODY}" >"${SB}/models/BirdNET_V3.onnx"
printf 'trunc' >"${SB}/models/labels.csv"
RC="$(run_download_model "${SB}")"
if grep -q 'labels.csv' "${SB}/calls.log"; then
    pass "the truncated labels file was re-fetched"
else
    fail "the truncated labels file was skipped"
fi
if grep -q 'BirdNET_V3.onnx' "${SB}/calls.log"; then
    fail "the good model was re-downloaded as well; only the bad file should move"
else
    pass "and the good model was left alone"
fi
[ "${RC}" = 0 ] || fail "download_model failed (exit ${RC})"
rm -rf "${SB}"

echo "=== 4. an absent model is fetched, which is the ordinary first install ==="
SB="$(mktemp -d)"; setup "${SB}"
RC="$(run_download_model "${SB}")"
if grep -q 'BirdNET_V3.onnx' "${SB}/calls.log" && grep -q 'labels.csv' "${SB}/calls.log"; then
    pass "both files were fetched"
else
    fail "a first install did not fetch both files"
    sed 's/^/        /' "${SB}/out.log" | head -5
fi
[ "${RC}" = 0 ] || fail "download_model failed (exit ${RC})"
rm -rf "${SB}"

if [ "${FAILED}" -eq 0 ]; then
    echo
    echo "model-resume: all pass"
else
    echo
    echo "model-resume: FAILURES"
fi
exit "${FAILED}"
