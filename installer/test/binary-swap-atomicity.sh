#!/usr/bin/env bash
# installer/test/binary-swap-atomicity.sh — the binary swap must never leave the
# path absent, short, or unrunnable, and must leave a way back.
#
# ## What was wrong
#
# `install_binary` ended with:
#
#     install -m 0755 "${extracted_binary}" "${INSTALL_DIR}/${BINARY_NAME}"
#
# `install` is not atomic and does not fsync. Traced with strace:
#
#     unlinkat(AT_FDCWD, "dst", 0)                            = 0
#     openat(AT_FDCWD, "src", O_RDONLY)                       = 3
#     openat(AT_FDCWD, "dst", O_WRONLY|O_CREAT|O_EXCL, 0600)  = 4
#
# The working binary is unlinked *first*, then a fresh file is created at the
# same path and filled. Writing ~100 MB to an SD card is a multi-second window,
# and an upgrade is exactly when a solar or battery-backed box browns out.
# Afterwards `ExecStartPre` and `ExecStart` both fail, `Restart=always` with
# `StartLimitIntervalSec=0` retries every five minutes for ever, there is no web
# UI left to say so, and the previous binary was deleted rather than kept.
#
# That last part also made the documented recovery impossible.
# `docs/book/field/deployment.md` tells operators to keep the previous binary at
# `.prev` "so a one-line `mv` rollback is possible"; nothing ever created it.
#
# ## What this gate holds
#
#   1. no `unlink`/`truncate` of the live path — the swap is a `rename`;
#   2. the previous binary survives at `.prev`;
#   3. a replacement that cannot run does not replace anything.
#
# (3) is the discrimination: a swap that renamed anything into place would
# satisfy (1) and (2) and still brick the station.
#
# Usage: installer/test/binary-swap-atomicity.sh
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BINARY_LIB="${HERE}/../lib/50-binary.sh"
FAILED=0

pass() { echo "  PASS  $*"; }
fail() { echo "  FAIL  $*"; FAILED=1; }

# Run the shipping `install_binary_atomically` against a sandbox.
#
#   run_swap <sandbox> <new-binary-body>
#
# Echoes its exit status; leaves output in ${sandbox}/out.log.
run_swap() {
    local sandbox="$1" body="$2"
    local src="${sandbox}/src/birdnet-behavior"
    mkdir -p "${sandbox}/src" "${sandbox}/bin"
    printf '%s' "${body}" >"${src}"
    chmod 0755 "${src}"

    (
        set -uo pipefail
        # The real function, from the real module, so this cannot pass against
        # a copy that has drifted from what ships.
        # shellcheck disable=SC1090
        source <(sed -n '/^install_binary_atomically()/,/^}/p' "${BINARY_LIB}")

        info()    { echo "[INFO] $*"; }
        success() { echo "[OK] $*"; }
        warn()    { echo "[WARN] $*"; }
        fatal()   { echo "[FATAL] $*"; exit 1; }

        # install.sh globals read by the `install_binary_atomically` body
        # sourced above — 50-binary.sh reads INSTALL_DIR and BINARY_NAME at 43.
        # Invisible to shellcheck: the body arrives through
        # `source <(sed -n ...)` and `-x` does not follow a process
        # substitution, so a global the sourced code reads looks unused here.
        # shellcheck disable=SC2034
        {
        BINARY_NAME="birdnet-behavior"
        INSTALL_DIR="${sandbox}/bin"
        SERVICE_NAME="birdnet-behavior.service"
        }

        install_binary_atomically "${src}"
    ) >"${sandbox}/out.log" 2>&1
    echo $?
}

# A runnable stub that prints the version it was given.
stub() { printf '#!/bin/sh\necho %s\n' "$1"; }

echo "=== 1. the live path is renamed into, never unlinked and refilled ==="
SB="$(mktemp -d)"
mkdir -p "${SB}/bin"
stub old >"${SB}/bin/birdnet-behavior"
chmod 0755 "${SB}/bin/birdnet-behavior"
OLD_INO="$(stat -c '%i' "${SB}/bin/birdnet-behavior")"

if command -v strace >/dev/null 2>&1; then
    # Watch the syscalls the swap actually makes against the live path.
    STRACE_LOG="${SB}/strace.log"
    mkdir -p "${SB}/src"
    stub new >"${SB}/src/birdnet-behavior"
    chmod 0755 "${SB}/src/birdnet-behavior"
    strace -f -o "${STRACE_LOG}" \
        -e trace=unlink,unlinkat,rename,renameat,renameat2,truncate,ftruncate \
        bash -c "
            source <(sed -n '/^install_binary_atomically()/,/^}/p' '${BINARY_LIB}')
            info() { :; }; success() { :; }; warn() { :; }; fatal() { exit 1; }
            BINARY_NAME=birdnet-behavior
            INSTALL_DIR='${SB}/bin'
            SERVICE_NAME=x.service
            install_binary_atomically '${SB}/src/birdnet-behavior'
        " >/dev/null 2>&1
    if grep -qE 'rename(at2?)?\(.*"'"${SB}"'/bin/birdnet-behavior"' "${STRACE_LOG}"; then
        pass "the live path is reached by rename(2)"
    else
        fail "no rename(2) onto the live path — the swap is not atomic"
        sed 's/^/        /' "${STRACE_LOG}" | head -20
    fi
    if grep -qE 'unlink(at)?\(.*"'"${SB}"'/bin/birdnet-behavior"' "${STRACE_LOG}"; then
        fail "the live binary was unlinked; a power cut here leaves no binary at all"
        grep -E 'unlink' "${STRACE_LOG}" | sed 's/^/        /' | head -5
    else
        pass "the live binary was never unlinked"
    fi
else
    # No strace: assert the observable consequence instead — the inode at the
    # live path changed (so it was replaced wholesale, not written through) and
    # the old inode is still reachable via .prev.
    echo "  NOTE  strace unavailable; asserting the observable consequence instead"
    RC="$(run_swap "${SB}" "$(stub new)")"
    [ "${RC}" = 0 ] || fail "the swap failed (exit ${RC})"
    NEW_INO="$(stat -c '%i' "${SB}/bin/birdnet-behavior")"
    if [ "${OLD_INO}" != "${NEW_INO}" ]; then
        pass "the live path points at a different inode — it was replaced, not refilled"
    else
        fail "the live path kept its inode; the new bytes were written through it"
    fi
fi
rm -rf "${SB}"

echo "=== 2. the previous binary is kept, so the documented rollback exists ==="
SB="$(mktemp -d)"
mkdir -p "${SB}/bin"
stub old >"${SB}/bin/birdnet-behavior"
chmod 0755 "${SB}/bin/birdnet-behavior"
RC="$(run_swap "${SB}" "$(stub new)")"
if [ "${RC}" != 0 ]; then
    fail "the swap failed (exit ${RC})"
    sed 's/^/        /' "${SB}/out.log"
fi
if [ -f "${SB}/bin/birdnet-behavior.prev" ]; then
    pass ".prev exists — docs/book/field/deployment.md's rollback is possible"
    if [ "$("${SB}/bin/birdnet-behavior.prev")" = "old" ]; then
        pass "and it is the binary that was there before"
    else
        fail ".prev is not the previous binary"
    fi
else
    fail "no .prev — the rollback the manual documents cannot be performed"
fi
if [ "$("${SB}/bin/birdnet-behavior")" = "new" ]; then
    pass "and the live path is the new binary"
else
    fail "the live path is not the new binary"
fi
if grep -q 'roll back with' "${SB}/out.log"; then
    pass "and the operator is told how to use it"
else
    fail "nothing told the operator .prev exists"
fi
rm -rf "${SB}"

echo "=== 3. a replacement that will not run does not replace anything ==="
SB="$(mktemp -d)"
mkdir -p "${SB}/bin"
stub old >"${SB}/bin/birdnet-behavior"
chmod 0755 "${SB}/bin/birdnet-behavior"
# A binary for the wrong architecture, or a truncated extraction, presents
# exactly like this: a file that exists and cannot execute.
RC="$(run_swap "${SB}" 'not an executable at all')"
if [ "${RC}" != 0 ]; then
    pass "the swap refused (exit ${RC})"
else
    fail "a binary that cannot run was installed anyway"
fi
if [ "$("${SB}/bin/birdnet-behavior" 2>/dev/null)" = "old" ]; then
    pass "and the working binary is untouched"
else
    fail "the working binary was replaced by one that cannot start"
    sed 's/^/        /' "${SB}/out.log"
fi
if ls "${SB}/bin"/*.new.* >/dev/null 2>&1; then
    fail "the staged file was left behind"
else
    pass "and the staged file was cleaned up"
fi
rm -rf "${SB}"

if [ "${FAILED}" -eq 0 ]; then
    echo
    echo "binary-swap-atomicity: all pass"
else
    echo
    echo "binary-swap-atomicity: FAILURES"
fi
exit "${FAILED}"
