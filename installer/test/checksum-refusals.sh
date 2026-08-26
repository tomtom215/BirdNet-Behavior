#!/usr/bin/env bash
# installer/test/checksum-refusals.sh — when the installer cannot verify what it
# downloaded, does it stop?
#
# Three places in the installer treated "we could not check this" as
# interchangeable with "this checked out":
#
#   1. install_binary: a failed SHA256SUMS fetch printed
#        [WARN] SHA256SUMS could not be downloaded — continuing without
#               checksum verification.
#      and installed the binary anyway. Whoever can substitute a 100 MB binary
#      can also drop one small request for the file that would expose it — so
#      the attacker, not the operator, decided whether verification happened.
#
#   2. install_binary again, more subtly: verification ran
#        sha256sum -c SHA256SUMS --ignore-missing --status --strict
#      which answers "did anything both listed and present mismatch?" — not
#      "did our archive verify?" Probed against GNU coreutils 9.4: with the
#      archive absent from SHA256SUMS but another listed file present and
#      matching, that command exits 0. The installer then printed "Checksum
#      verified against SHA256SUMS" having verified something else.
#
#   3. verify_model_sha256: a missing sha256sum returned 0 — the same value a
#      verified file returns — so the absence of the checking tool counted as
#      a successful check of the 541 MB model.
#
# `CLAUDE.md` names this shape, and D13 closed an instance of it in CI. These
# are the installer's instances.
#
# Usage: installer/test/checksum-refusals.sh
# Needs only bash + coreutils. Exit 0 = all pass.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
FAILED=0

pass() { printf '  PASS  %s\n' "$*"; }
fail() { printf '  FAIL  %s\n' "$*"; FAILED=1; }

BINARY_LIB="${REPO_ROOT}/installer/lib/50-binary.sh"
MODEL_LIB="${REPO_ROOT}/installer/lib/55-model.sh"

# ---------------------------------------------------------------------------
# Harness
#
# Each scenario runs install_binary as it ships, in a subshell, with `download`
# stubbed to produce the situation under test. Everything else — the workdir,
# the awk narrowing, the sha256sum call, the fatal — is the real code.
# ---------------------------------------------------------------------------

# run_install_binary <download-stub-body>
#
# The stub is called as `download URL DEST`. Writes combined output to
# ${OUT} and returns install_binary's exit status.
OUT=""
run_install_binary() {
    local stub_body="$1"
    local sandbox
    sandbox="$(mktemp -d)"
    OUT="${sandbox}/out.log"

    (
        set -euo pipefail
        # The real function, sourced from the real module, so this test cannot
        # pass against a copy that has drifted from what ships.
        # shellcheck disable=SC1090
        source <(sed -n '/^install_binary()/,/^}/p' "${BINARY_LIB}")

        info()    { echo "[INFO] $*"; }
        success() { echo "[OK] $*"; }
        warn()    { echo "[WARN] $*"; }
        fatal()   { echo "[FATAL] $*"; exit 1; }
        # install_binary stops the service immediately before the swap (see
        # 77-manage.sh); there is none here.
        stop_running_service_for_swap() { echo "[STOP] service stop requested"; }

        eval "${stub_body}"

        # These are install.sh globals (10-config.sh) that install_binary
        # reads. shellcheck cannot see the use because the function arrives
        # through a process substitution, which is the whole point — the test
        # drives the shipping code, not a copy.
        # shellcheck disable=SC2034
        BINARY_NAME="birdnet-behavior"
        # shellcheck disable=SC2034
        REPO="tomtom215/BirdNet-Behavior"
        INSTALL_DIR="${sandbox}/bin"
        # shellcheck disable=SC2034
        HELP_DIR="${sandbox}/help"
        mkdir -p "${INSTALL_DIR}"

        install_binary 9.9.9 x86_64
    ) >"${OUT}" 2>&1
    local rc=$?
    echo "${sandbox}" >"${OUT}.sandbox"
    return "${rc}"
}

# A stub that writes a fake archive, then whatever the caller wants for the
# SHA256SUMS request. `$2` is the destination path.
ARCHIVE="birdnet-behavior-9.9.9-x86_64.tar.gz"

# Build a real, extractable tarball at $1 containing dir/birdnet-behavior, so
# the success path can run all the way through to `install`.
make_real_tarball() {
    local dest="$1" staging
    staging="$(mktemp -d)"
    mkdir -p "${staging}/birdnet-behavior-9.9.9-x86_64"
    printf '#!/bin/sh\necho 9.9.9\n' \
        >"${staging}/birdnet-behavior-9.9.9-x86_64/birdnet-behavior"
    chmod 0755 "${staging}/birdnet-behavior-9.9.9-x86_64/birdnet-behavior"
    tar -czf "${dest}" -C "${staging}" birdnet-behavior-9.9.9-x86_64
    rm -rf "${staging}"
}
export -f make_real_tarball

echo "=== 1. a SHA256SUMS that cannot be fetched stops the install ==="
run_install_binary '
    download() {
        case "$1" in
            *SHA256SUMS) return 1 ;;            # the fetch fails
            *) make_real_tarball "$2" ;;
        esac
    }
'
rc=$?
if [ "${rc}" -ne 0 ]; then
    pass "install_binary aborted (exit ${rc})"
else
    fail "install_binary returned 0 — it installed a binary it could not verify"
fi
if grep -q "continuing without checksum verification" "${OUT}"; then
    fail "the old warn-and-continue message is still being printed"
else
    pass "no 'continuing without checksum verification'"
fi
if grep -q "Checksum verified" "${OUT}"; then
    fail "it claimed a checksum was verified with no SHA256SUMS at all"
else
    pass "it made no verification claim"
fi
sandbox="$(cat "${OUT}.sandbox")"
if [ -e "${sandbox}/bin/birdnet-behavior" ]; then
    fail "an unverified binary was installed to INSTALL_DIR"
else
    pass "nothing was written to INSTALL_DIR"
fi
# The station stays up. Making verification fatal is only an improvement if a
# failed update leaves the service running: do_install used to stop it before
# install_binary was even called, so every new fatal path here would have taken
# a working station off the air for a binary that was never installed.
if grep -q "\[STOP\]" "${OUT}"; then
    fail "the running service was stopped for an update that then refused to install"
else
    pass "the running service was never stopped"
fi

echo
echo "=== 2. a SHA256SUMS with no line for OUR archive stops the install ==="
# This is the --ignore-missing trap, reproduced exactly: SHA256SUMS names a
# different file, that file is present in the workdir, and it matches. GNU
# sha256sum -c --ignore-missing exits 0 on this input.
run_install_binary '
    download() {
        case "$1" in
            *SHA256SUMS)
                local dir; dir="$(dirname "$2")"
                printf "decoy\n" >"${dir}/uninstall.sh"
                (cd "${dir}" && sha256sum uninstall.sh) >"$2"
                ;;
            *) make_real_tarball "$2" ;;
        esac
    }
'
rc=$?
if [ "${rc}" -ne 0 ]; then
    pass "install_binary aborted (exit ${rc})"
else
    fail "install_binary returned 0 — sha256sum -c said OK about a file that was not the archive"
fi
if grep -q "Checksum verified" "${OUT}"; then
    fail "it printed 'Checksum verified' without verifying the archive"
else
    pass "it made no verification claim"
fi
if grep -q "no entry for ${ARCHIVE}" "${OUT}"; then
    pass "and said which entry was missing"
else
    fail "the abort message does not name the missing entry"
fi

echo
echo "=== 3. a SHA256SUMS whose digest does not match stops the install ==="
run_install_binary '
    download() {
        case "$1" in
            *SHA256SUMS)
                printf "%s  %s\n" "$(printf 0 | tr 0 a | head -c1)$(yes a | head -63 | tr -d "\n")" \
                    "birdnet-behavior-9.9.9-x86_64.tar.gz" >"$2"
                ;;
            *) make_real_tarball "$2" ;;
        esac
    }
'
rc=$?
if [ "${rc}" -ne 0 ]; then
    pass "install_binary aborted (exit ${rc})"
else
    fail "a mismatched digest was accepted"
fi

echo
echo "=== 4. counterpart: a correct SHA256SUMS still installs ==="
# Without this, everything above is satisfied by an installer that refuses
# unconditionally.
run_install_binary '
    download() {
        case "$1" in
            *SHA256SUMS)
                local dir; dir="$(dirname "$2")"
                (cd "${dir}" && sha256sum birdnet-behavior-9.9.9-x86_64.tar.gz) >"$2"
                ;;
            *) make_real_tarball "$2" ;;
        esac
    }
'
rc=$?
if [ "${rc}" -eq 0 ]; then
    pass "install_binary completed (exit 0)"
else
    fail "a correctly-verified archive was refused (exit ${rc}) — output follows"
    sed 's/^/        /' "${OUT}"
fi
if grep -q "Checksum verified against SHA256SUMS" "${OUT}"; then
    pass "and said so"
else
    fail "no verification message on the success path"
fi
# The counterpart to the "never stopped" check above: on the path that *does*
# install, the service must still be stopped first, or the swap hits ETXTBSY.
if grep -q "\[STOP\]" "${OUT}"; then
    pass "and stopped the running service before swapping the binary"
else
    fail "the binary was swapped without stopping the running service (ETXTBSY)"
fi
sandbox="$(cat "${OUT}.sandbox")"
if [ -x "${sandbox}/bin/birdnet-behavior" ]; then
    pass "the binary was installed"
else
    fail "the happy path did not install the binary"
fi

echo
echo "=== 5. counterpart: a directory-prefixed SHA256SUMS line still verifies ==="
# `sha256sum -c` looks a path up exactly as written, so a `release/<archive>`
# entry would report a missing file unless the line is normalised to the bare
# name. Probed: an unnormalised prefixed line exits 1 with "No such file".
run_install_binary '
    download() {
        case "$1" in
            *SHA256SUMS)
                local dir; dir="$(dirname "$2")"
                (cd "${dir}" && sha256sum birdnet-behavior-9.9.9-x86_64.tar.gz) \
                    | sed "s#  #  release/#" >"$2"
                ;;
            *) make_real_tarball "$2" ;;
        esac
    }
'
rc=$?
if [ "${rc}" -eq 0 ]; then
    pass "a release/-prefixed entry verified against the local archive"
else
    fail "a directory-prefixed entry was not normalised (exit ${rc})"
    sed 's/^/        /' "${OUT}"
fi

echo
echo "=== 6. the model checker treats a missing sha256sum as a refusal ==="
model_out="$(mktemp)"
(
    set -uo pipefail
    # shellcheck disable=SC1090
    source <(sed -n '/^verify_model_sha256()/,/^}/p' "${MODEL_LIB}")
    warn()  { echo "[WARN] $*"; }
    error() { echo "[ERROR] $*"; }
    fatal() { echo "[FATAL] $*"; exit 1; }
    # An empty PATH is the only honest way to make `command -v sha256sum` fail.
    # shellcheck disable=SC2123
    PATH=""
    verify_model_sha256 /nonexistent/BirdNET_GLOBAL_6K_V3.0_Model_FP32.onnx deadbeef
) >"${model_out}" 2>&1
rc=$?
if [ "${rc}" -ne 0 ]; then
    pass "verify_model_sha256 refused (exit ${rc})"
else
    fail "verify_model_sha256 returned 0 — 'cannot check' passed as 'checked'"
fi
if grep -q "BirdNET_GLOBAL_6K_V3.0_Model_FP32.onnx" "${model_out}"; then
    pass "and named the file, without needing basename on an empty PATH"
else
    fail "the abort message lost the filename: $(cat "${model_out}")"
fi
rm -f "${model_out}"

echo
echo "=== 7. counterpart: a matching model digest is still accepted ==="
model_out="$(mktemp)"
sample="$(mktemp)"
printf 'model bytes\n' >"${sample}"
expected="$(sha256sum "${sample}" | awk '{print $1}')"
(
    set -uo pipefail
    # shellcheck disable=SC1090
    source <(sed -n '/^verify_model_sha256()/,/^}/p' "${MODEL_LIB}")
    warn()  { echo "[WARN] $*"; }
    error() { echo "[ERROR] $*"; }
    fatal() { echo "[FATAL] $*"; exit 1; }
    verify_model_sha256 "${sample}" "${expected}"
) >"${model_out}" 2>&1
rc=$?
if [ "${rc}" -eq 0 ]; then
    pass "a matching digest verifies"
else
    fail "a matching digest was rejected (exit ${rc}): $(cat "${model_out}")"
fi
(
    set -uo pipefail
    # shellcheck disable=SC1090
    source <(sed -n '/^verify_model_sha256()/,/^}/p' "${MODEL_LIB}")
    warn()  { echo "[WARN] $*"; }
    error() { echo "[ERROR] $*"; }
    fatal() { echo "[FATAL] $*"; exit 1; }
    verify_model_sha256 "${sample}" "$(printf 'f%.0s' {1..64})"
) >"${model_out}" 2>&1
rc=$?
if [ "${rc}" -ne 0 ]; then
    pass "a mismatched digest is rejected"
else
    fail "a mismatched digest was accepted"
fi
rm -f "${model_out}" "${sample}"

echo
if [ "${FAILED}" -eq 0 ]; then
    echo "checksum-refusals: all pass"
else
    echo "checksum-refusals: FAILURES"
fi
exit "${FAILED}"
