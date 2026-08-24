#!/usr/bin/env bash
# installer/test/run-ci.sh — run the installer test suite that CI can run, and
# refuse to let a test go unrun by accident.
#
# Why this exists: installer/test/ held five test scripts and *nothing executed
# any of them* — not a workflow, not a script, not a Makefile. Four of the five
# still passed when finally run by hand, so nothing had rotted yet, but a test
# nobody runs is a test that stops being true silently. `installer/build.sh
# --check` was the only installer gate wired into CI.
#
# The accounting below is the part that matters. Listing the tests to run would
# have the same failure mode one level up: add a sixth test, forget to add it
# here, and it joins the four that were never run. So every file in
# installer/test/ must appear in exactly one of CI_TESTS or EXCLUDED — and
# EXCLUDED entries carry the reason in the same breath. An unaccounted file is
# a failure, not a silent omission.
#
# Usage: installer/test/run-ci.sh
# Exit 0 = every CI test passed and every file is accounted for.

set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FAILED=0

# Hermetic: bash + coreutils, no network, no docker, no root.
CI_TESTS=(
    admin-login-summary.sh
    alsa-device-detect.sh
    checksum-refusals.sh
    coord-parse.sh
    location-notice.sh
    pipefail-sigpipe.sh
    service-unit.sh
)

# Deliberately not run in CI. Each entry is "file|reason" and the reason is
# shown on every run, so an exclusion has to keep justifying itself rather than
# quietly becoming permanent.
EXCLUDED=(
    "pkg-manager.sh|needs docker and four distro mirror networks; run it by hand when touching 30-platform.sh"
    "run-ci.sh|this runner"
)

# ── accounting ──────────────────────────────────────────────────────────────
echo "=== accounting: every test file is either run or excluded with a reason ==="
for path in "${HERE}"/*.sh; do
    file="$(basename "${path}")"
    found=0
    for t in "${CI_TESTS[@]}"; do
        [ "${t}" = "${file}" ] && found=1
    done
    for e in "${EXCLUDED[@]}"; do
        [ "${e%%|*}" = "${file}" ] && found=$((found + 1))
    done
    case "${found}" in
        1) ;;
        0)
            printf '  FAIL  %s is in installer/test/ but neither run nor excluded.\n' "${file}"
            printf '        Add it to CI_TESTS, or to EXCLUDED with a reason.\n'
            FAILED=1
            ;;
        *)
            printf '  FAIL  %s is listed twice — it cannot be both run and excluded.\n' "${file}"
            FAILED=1
            ;;
    esac
done
for t in "${CI_TESTS[@]}"; do
    if [ ! -f "${HERE}/${t}" ]; then
        printf '  FAIL  CI_TESTS names %s, which does not exist.\n' "${t}"
        FAILED=1
    fi
done
[ "${FAILED}" -eq 0 ] && printf '  PASS  all %d file(s) accounted for\n' \
    "$(find "${HERE}" -maxdepth 1 -name '*.sh' | wc -l)"

for e in "${EXCLUDED[@]}"; do
    [ "${e%%|*}" = "run-ci.sh" ] && continue
    printf '  SKIP  %s — %s\n' "${e%%|*}" "${e#*|}"
done

# ── run ─────────────────────────────────────────────────────────────────────
for t in "${CI_TESTS[@]}"; do
    echo
    echo "############ ${t}"
    if bash "${HERE}/${t}"; then
        printf '############ %s: OK\n' "${t}"
    else
        printf '############ %s: FAILED (exit %d)\n' "${t}" "$?"
        FAILED=1
    fi
done

echo
if [ "${FAILED}" -eq 0 ]; then
    echo "installer test suite: all pass"
else
    echo "installer test suite: FAILURES"
fi
exit "${FAILED}"
