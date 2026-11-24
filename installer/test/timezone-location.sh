#!/usr/bin/env bash
# installer/test/timezone-location.sh — does the installer notice a machine
# whose time zone is on another continent from the coordinates just typed?
#
# This exists because of audit finding OOB-06. The installer never read or set
# a time zone, and capture names every recording from the machine's local time,
# so a Pi left on Raspberry Pi OS's default Europe/London in Ohio filed its dawn
# chorus five hours late. A location typed at the prompt leaves the wizard no
# zone to look up, so `--doctor` passed it too. The prompt now compares the
# machine's UTC offset with the longitude and warns; it never stops.
#
# The margin (3.5 hours against longitude / 15) is the doctor's; the doctor's
# doc comment says why. The pass cases below are the zones that sit far from
# the sun on purpose and must not be told they are wrong.
#
# Usage: installer/test/timezone-location.sh
# Needs only bash + awk. Exit 0 = all pass.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
FAILED=0
# Run from an empty directory and check it stays empty: in awk, a bare `>` in a
# print statement's arguments is a redirection, and a draft of the check wrote
# a file called "behind" into the installer's working directory.
WORK="$(mktemp -d)"
trap 'rm -rf "${WORK}"' EXIT
cd "${WORK}" || exit 1

pass() { printf '  PASS  %s\n' "$*"; }
fail() { printf '  FAIL  %s\n' "$*"; FAILED=1; }

# Load the real functions out of the real module, so this cannot pass against a
# copy that has drifted from what ships.
LIB="${REPO_ROOT}/installer/lib/70-station.sh"
# shellcheck disable=SC1090
source <(sed -n '/^timezone_location_mismatch()/,/^}/p' "${LIB}")
# shellcheck disable=SC1090
source <(sed -n '/^warn_if_timezone_mismatches_location()/,/^}/p' "${LIB}")
# shellcheck disable=SC1090
source <(sed -n '/^host_timezone_name()/,/^}/p' "${LIB}")

echo "=== 0. the installer has the check at all ==="
for fn in timezone_location_mismatch warn_if_timezone_mismatches_location host_timezone_name; do
    if declare -F "${fn}" >/dev/null; then
        pass "${fn} is defined in 70-station.sh"
    else
        fail "${fn} is not defined in 70-station.sh — the installer never compares the zone with the location"
    fi
done

# warns <lat> <lon> <offset> <zone> <description>
warns() {
    local out
    if out="$(timezone_location_mismatch "$1" "$2" "$3" "$4" 2>&1)" \
        && grep -q "$4" <<<"${out}" \
        && grep -q 'timedatectl set-timezone' <<<"${out}" \
        && grep -q 'wrong hours' <<<"${out}"; then
        pass "$5"
    else
        fail "$5 — no warning naming $4 and the fix (got: '${out}')"
    fi
}

# quiet <lat> <lon> <offset> <zone> <description>
quiet() {
    local out
    if out="$(timezone_location_mismatch "$1" "$2" "$3" "$4" 2>&1)"; then
        fail "$5 — a correct zone was told it is wrong: '${out}'"
    elif [ -n "${out}" ]; then
        fail "$5 — no mismatch, but it printed '${out}'"
    else
        pass "$5"
    fi
}

echo
echo "=== 1. the gate: Raspberry Pi OS's default zone on another continent ==="
warns 40.0 -83.0 +0000 Europe/London "Europe/London (GMT) in Ohio warns"
warns 40.0 -83.0 +0100 Europe/London "Europe/London (BST) in Ohio warns"
warns -33.87 151.21 +0000 UTC "UTC in Sydney warns"
warns 28.61 77.21 +0000 Europe/London "Europe/London in Delhi warns"
warns 1.87 -157.43 +0000 Europe/London "Europe/London on Kiritimati warns (the wrap goes both ways)"
out="$(timezone_location_mismatch 40.0 -83.0 +0000 Europe/London)"
grep -q 'UTC-6' <<<"${out}" \
    && pass "…and names the offset the location implies (UTC-6)" \
    || fail "…but does not say which offset the location implies: '${out}'"

echo
echo "=== 2. counterpart: the right zone, and zones far from the sun on purpose ==="
quiet 40.0 -83.0 -0500 America/New_York "America/New_York (EST) in Ohio is quiet"
quiet 40.0 -83.0 -0400 America/New_York "America/New_York (EDT) in Ohio is quiet"
quiet 43.8 87.6 +0800 Asia/Shanghai "Asia/Shanghai in Urumqi (2.2 h) is quiet"
quiet 39.47 75.99 +0800 Asia/Shanghai "Asia/Shanghai in Kashgar (2.9 h) is quiet"
quiet 40.42 -3.70 +0200 Europe/Madrid "Europe/Madrid in summer in Madrid is quiet"
quiet 42.88 -8.54 +0200 Europe/Madrid "Europe/Madrid in summer in Santiago de Compostela is quiet"
quiet -32.89 -68.83 -0300 America/Argentina/Mendoza "Argentina's UTC-3 in Mendoza is quiet"
quiet 64.50 -165.41 -0800 America/Anchorage "America/Anchorage in summer in Nome (3.0 h) is quiet"
quiet 1.87 -157.43 +1400 Pacific/Kiritimati "Kiribati's UTC+14 at 157 W is quiet"
quiet 23.02 72.57 +0530 Asia/Kolkata "a half-hour zone (India) is quiet"
quiet 27.72 85.32 +0545 Asia/Kathmandu "a 45-minute zone (Nepal) is quiet"
quiet 40.0 banana +0000 UTC "an unreadable longitude is not judged"

echo
echo "=== 3. the prompt calls it, and it cannot stop the installer ==="
body="$(sed -n '/^prompt_station_settings()/,/^}/p' "${LIB}")"
if grep -A3 'success "Location:' <<<"${body}" | grep -q 'warn_if_timezone_mismatches_location'; then
    pass "prompt_station_settings checks the zone right after the coordinates are accepted"
else
    fail "prompt_station_settings never calls warn_if_timezone_mismatches_location"
fi
# The installer runs under `set -euo pipefail`; "no mismatch" is exit 1 from
# the comparison, and must not end the install. `date` is replaced so the
# machine running this test does not decide the outcome.
for case in "0.0 0.0 continues" "40.0 -83.0 warns"; do
    read -r lat lon expect <<<"${case}"
    set -- "${lat}" "${lon}" "${expect}"
    out="$(LIB="${LIB}" bash -c '
        set -euo pipefail
        source <(sed -n "/^timezone_location_mismatch()/,/^}/p" "$LIB")
        source <(sed -n "/^warn_if_timezone_mismatches_location()/,/^}/p" "$LIB")
        date() { echo "+0000"; }
        host_timezone_name() { echo "Europe/London"; }
        warn() { echo "[WARN] $*"; }
        warn_if_timezone_mismatches_location "$1" "$2"
        echo "still running"
    ' _ "$1" "$2" 2>&1)"
    if ! grep -q 'still running' <<<"${out}"; then
        fail "($1, $2) under set -euo pipefail: the installer stopped: '${out}'"
    elif [ "$3" = warns ] && ! grep -q '^\[WARN\] This machine.s clock is set to Europe/London' <<<"${out}"; then
        fail "($1, $2): no warning through warn(): '${out}'"
    elif [ "$3" = continues ] && grep -q WARN <<<"${out}"; then
        fail "($1, $2): warned about a matching zone: '${out}'"
    else
        pass "($1, $2) under set -euo pipefail: $3"
    fi
done

if [ -z "$(ls -A "${WORK}")" ]; then
    pass "the check wrote no files"
else
    fail "the check wrote files into the working directory: $(ls -A "${WORK}")"
fi

echo
if [ "${FAILED}" -eq 0 ]; then
    echo "timezone-location: ALL-PASS"
else
    echo "timezone-location: FAILURES"
fi
exit "${FAILED}"
