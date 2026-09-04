#!/usr/bin/env bash
# installer/test/container-model-cache.sh — does the container re-verify a model
# it finds already cached?
#
# `docker/entrypoint.sh`'s `ensure_model_file` is the container's whole model
# acquisition path. Its header says it will "accept the result only once it
# matches EXPECTED sha256". That is true of the download branch and was not true
# of the branch three lines below the comment:
#
#     if [ -f "$dest" ]; then
#         actual="$(wc -c < "$dest" ...)"
#         log "${desc}: already cached (...) — skipping download."
#         return 0
#     fi
#
# Presence, not verification: `$expected` was passed in and never consulted, so
# any unverified file sitting at that path was adopted as the model for the life
# of the volume. It does not take a partial download to get one there —
# `fetch_one` stages at `${dest}.tmp` and only moves on success — but a
# container killed between that move and the verification, a volume restored
# from a truncated snapshot, or a disk that filled during the move all leave
# exactly this, and every later start took it. Nothing downstream catches it:
# `src/doctor/model.rs` accepts any file over one megabyte, and the compose
# healthcheck polls `/api/v2/health` without `?strict=1`, which answers 200
# while its own body says the detection daemon is stopped.
#
# `installer/lib/55-model.sh` fixed exactly this on bare metal, with
# `model_file_is_verified`. The container was never brought along. It is the
# unfixed half of finding LC-2.
#
# The second scenario here is `verify_sha256` itself, which returned 0 — the
# same value a verified file returns — when `sha256sum` was absent. That is
# finding LC-15, and it is the shape `checksum-refusals.sh` exists for: "we
# could not check this" must never be interchangeable with "this checked out".
# The installer closed all three of its instances; the container kept this one.
#
# Usage: installer/test/container-model-cache.sh
# Needs only bash + coreutils. Exit 0 = all pass.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
ENTRYPOINT="${REPO_ROOT}/docker/entrypoint.sh"
FAILED=0

pass() { printf '  PASS  %s\n' "$*"; }
fail() { printf '  FAIL  %s\n' "$*"; FAILED=1; }

# ---------------------------------------------------------------------------
# Harness
#
# The functions under test are sourced out of the shipping entrypoint by name,
# the same way checksum-refusals.sh drives the installer, so this cannot pass
# against a copy that has drifted from what runs in the container.
# ---------------------------------------------------------------------------
load_entrypoint_fns() {
    # shellcheck disable=SC1090
    source <(sed -n '/^verify_sha256()/,/^}/p' "${ENTRYPOINT}")
    # shellcheck disable=SC1090
    source <(sed -n '/^human_bytes()/,/^}/p' "${ENTRYPOINT}")
    # shellcheck disable=SC1090
    source <(sed -n '/^ensure_model_file()/,/^}/p' "${ENTRYPOINT}")
}

DIGEST_OF_GOOD=""
SANDBOX=""

# run_ensure <cached-contents> <expected-digest> — returns ensure_model_file's
# status; combined output in ${OUT}; whether a download was attempted in
# ${SANDBOX}/fetched.
run_ensure() {
    local contents="$1" expected="$2"
    SANDBOX="$(mktemp -d)"
    OUT="${SANDBOX}/out.log"
    (
        set -uo pipefail
        log()  { echo "[birdnet] $*"; }
        warn() { echo "[birdnet] WARNING: $*"; }
        die()  { echo "[birdnet] ERROR: $*"; exit 1; }
        load_entrypoint_fns
        # A download stub that records that it ran and writes a good file, so a
        # re-download after a failed verification succeeds — which is what
        # should happen, and what distinguishes "refuse" from "recover".
        fetch_one() {
            echo "fetched" >>"${SANDBOX}/fetched"
            printf 'GOOD MODEL BYTES' >"$1"
            return 0
        }
        # Read by `ensure_model_file` when it names the origin it fetched
        # from. shellcheck cannot see the use because the function arrives
        # through a process substitution, which is the whole point — the test
        # drives the shipping code, not a copy.
        # shellcheck disable=SC2034
        MODEL_RELEASE_TAG="test"
        printf '%s' "${contents}" >"${SANDBOX}/model.onnx"
        ensure_model_file "${SANDBOX}/model.onnx" \
            "https://example.invalid/gh" "https://example.invalid/zenodo" \
            "${expected}" "Model"
    ) >"${OUT}" 2>&1
    return $?
}

DIGEST_OF_GOOD="$(printf 'GOOD MODEL BYTES' | sha256sum | awk '{print $1}')"

echo "=== a cached file that does not match its digest must not be adopted ==="
if run_ensure 'TRUNCATED' "${DIGEST_OF_GOOD}"; then
    if [ -s "${SANDBOX}/fetched" ] && \
       [ "$(cat "${SANDBOX}/model.onnx")" = "GOOD MODEL BYTES" ]; then
        pass "a mismatched cached file was discarded and re-fetched"
    else
        fail "ensure_model_file accepted a cached file it never verified"
        printf '        cached bytes kept: %s\n' "$(cat "${SANDBOX}/model.onnx")"
        sed 's/^/        /' "${OUT}"
    fi
else
    # Refusing outright is also acceptable — but only if it did not silently
    # succeed. Getting here means it died, which is a safe answer.
    pass "a mismatched cached file was refused"
fi

echo "=== the counterpart: a cached file that DOES match is used, not re-fetched ==="
if run_ensure 'GOOD MODEL BYTES' "${DIGEST_OF_GOOD}"; then
    if [ -e "${SANDBOX}/fetched" ]; then
        fail "a verified cached model was downloaded again — every restart would re-fetch 541 MB"
        sed 's/^/        /' "${OUT}"
    else
        pass "a verified cached model is used without downloading"
    fi
else
    fail "a verified cached model was rejected"
    sed 's/^/        /' "${OUT}"
fi

echo "=== verify_sha256 must not report success when it cannot check ==="
rc=0
(
    set -uo pipefail
    warn() { echo "[birdnet] WARNING: $*"; }
    load_entrypoint_fns
    # Hide sha256sum from the function without touching the filesystem: an
    # empty PATH plus bash's own builtins is what a stripped image looks like.
    sha256sum() { return 127; }
    command() {
        if [ "${2:-}" = "sha256sum" ]; then return 1; fi
        builtin command "$@"
    }
    tmp="$(mktemp)"; printf 'anything' >"$tmp"
    verify_sha256 "$tmp" "0000000000000000000000000000000000000000000000000000000000000000"
) >/dev/null 2>&1 || rc=$?
if [ "${rc}" -eq 0 ]; then
    fail "verify_sha256 returned success with no sha256sum available — 'could not check' must not equal 'checked out'"
else
    pass "verify_sha256 refuses when it cannot check"
fi

echo
if [ "${FAILED}" -ne 0 ]; then
    echo "container-model-cache.sh: FAILED"
    exit 1
fi
echo "container-model-cache.sh: all pass"
