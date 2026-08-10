#!/usr/bin/env bash
# installer/test/alsa-device-detect.sh — drive detect_first_audio_device()
# against captured `arecord -l` output.
#
# The function picks the device string the installer writes into
# /etc/birdnet/birdnet.conf, and that string decides whether a station still
# records after a reboot. It shells out to arecord, so it is tested by putting
# a fake arecord on PATH that prints a fixture — no sound hardware needed, and
# the fixtures are real listings rather than invented ones.
#
# Fixtures 1 and 2 are the same microphone on the same Raspberry Pi 4, captured
# either side of a cold reboot during the acceptance run: card index 1 before,
# 3 after, id `PRO` both times. They are the whole argument for addressing the
# card by id, and the test asserts the two produce an IDENTICAL device string.
#
# Usage:  installer/test/alsa-device-detect.sh
# Needs only bash + awk. Exit 0 = all pass.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK="$(mktemp -d)"
trap 'rm -rf "${WORK}"' EXIT
FAILED=0

pass() { printf '  PASS  %s\n' "$*"; }
fail() { printf '  FAIL  %s\n' "$*"; FAILED=1; }

# ── Fixtures ────────────────────────────────────────────────────────────────

mkfixture() { mkdir -p "${WORK}/fx"; cat > "${WORK}/fx/$1"; }

# Captured verbatim from the Pi 4, BEFORE the cold reboot.
mkfixture pi_before <<'EOF'
**** List of CAPTURE Hardware Devices ****
card 1: PRO [Comica_Traxshot PRO], device 0: USB Audio [USB Audio]
  Subdevices: 1/1
  Subdevice #0: subdevice #0
EOF

# Captured verbatim from the SAME Pi and SAME microphone, AFTER the reboot.
mkfixture pi_after <<'EOF'
**** List of CAPTURE Hardware Devices ****
card 3: PRO [Comica_Traxshot PRO], device 0: USB Audio [USB Audio]
  Subdevices: 1/1
  Subdevice #0: subdevice #0
EOF

# Two identical microphones: the id no longer identifies one card, so the
# index is the only thing that can distinguish them here.
mkfixture dup_ids <<'EOF'
**** List of CAPTURE Hardware Devices ****
card 1: PRO [Comica_Traxshot PRO], device 0: USB Audio [USB Audio]
  Subdevices: 1/1
card 2: PRO [Comica_Traxshot PRO], device 0: USB Audio [USB Audio]
  Subdevices: 1/1
EOF

# An id carrying characters we will not emit unquoted into a config value.
mkfixture odd_id <<'EOF'
**** List of CAPTURE Hardware Devices ****
card 1: My.Odd$Card [Weird Device], device 0: USB Audio [USB Audio]
  Subdevices: 1/1
EOF

# No capture hardware at all — what a station with the mic unplugged shows.
mkfixture no_cards <<'EOF'
**** List of CAPTURE Hardware Devices ****
EOF

# A capture device that is not device 0.
mkfixture dev_nonzero <<'EOF'
**** List of CAPTURE Hardware Devices ****
card 2: Scarlett [Focusrite Scarlett 2i2], device 2: USB Audio [USB Audio]
  Subdevices: 1/1
EOF

# Two different microphones: first listed wins, and its id is unambiguous.
mkfixture multi_distinct <<'EOF'
**** List of CAPTURE Hardware Devices ****
card 1: PRO [Comica_Traxshot PRO], device 0: USB Audio [USB Audio]
  Subdevices: 1/1
card 2: Scarlett [Focusrite Scarlett 2i2], device 0: USB Audio [USB Audio]
  Subdevices: 1/1
EOF

# ── Runner ──────────────────────────────────────────────────────────────────

# run_detect <lib-dir> <fixture> -> prints the device string the installer
# would write. Each call is a fresh bash so sourcing one implementation cannot
# leak into another, which matters for the A/B below.
run_detect() {
    local libdir="$1" fixture="$2"
    local bindir="${WORK}/bin.$$"
    mkdir -p "${bindir}"
    printf '#!/usr/bin/env bash\ncat %q\n' "${WORK}/fx/${fixture}" > "${bindir}/arecord"
    chmod +x "${bindir}/arecord"
    PATH="${bindir}:${PATH}" bash -c "
        set -u
        . '${libdir}/10-config.sh'
        . '${libdir}/20-log.sh'
        . '${libdir}/30-platform.sh'
        . '${libdir}/70-station.sh'
        set +e
        detect_first_audio_device
    " 2>/dev/null
    rm -rf "${bindir}"
}

expect() { # expect <fixture> <expected> <description>
    local got
    got="$(run_detect "${REPO_ROOT}/installer/lib" "$1")"
    if [ "${got}" = "$2" ]; then
        pass "$3 -> '${got}'"
    else
        fail "$3 -> got '${got}', expected '$2'"
    fi
}

echo "=== detect_first_audio_device, current implementation ==="
expect pi_before      "plughw:CARD=PRO,DEV=0"      "Pi 4 before reboot (card 1)"
expect pi_after       "plughw:CARD=PRO,DEV=0"      "Pi 4 after reboot (card 3)"
expect dup_ids        "plughw:1,0"                 "two identical mics: index fallback"
expect odd_id         "plughw:1,0"                 "unportable id: index fallback"
expect no_cards       ""                           "no capture hardware"
expect dev_nonzero    "plughw:CARD=Scarlett,DEV=2" "non-zero device index preserved"
expect multi_distinct "plughw:CARD=PRO,DEV=0"      "two distinct mics: first, by id"

echo "=== the property that matters: stable across re-enumeration ==="
before="$(run_detect "${REPO_ROOT}/installer/lib" pi_before)"
after="$(run_detect "${REPO_ROOT}/installer/lib" pi_after)"
if [ -n "${before}" ] && [ "${before}" = "${after}" ]; then
    pass "same string either side of the reboot ('${before}')"
else
    fail "reboot changed the device string: '${before}' -> '${after}'"
fi

# ── Counter-test ────────────────────────────────────────────────────────────
# A test that only exercises the new code cannot show the bug was real, so the
# index-based implementation is kept here as a fixture and run over the same
# listings. It must FAIL the stability property above; if it ever passes, the
# fixtures no longer describe the defect and the test above proves nothing.
#
# Deliberately embedded rather than fetched from git history. The first version
# of this read `HEAD~1:installer/lib/70-station.sh`, which broke the moment
# another commit landed on top: HEAD~1 then already carried the fix, the
# "previous" implementation was the new one, and the counter-test reported that
# the change fixed nothing. A test whose meaning depends on its position in
# history is a test that rots.
legacy_detect() {
    local fixture="$1" first_card first_device listing
    listing="$(cat "${WORK}/fx/${fixture}")"
    first_card="$(printf '%s\n' "${listing}" | awk '/^card/{print $2; exit}' | tr -d ':')"
    first_device="$(printf '%s\n' "${listing}" \
        | awk '/^card/{ if (match($0, /device [0-9]+/)) print substr($0, RSTART + 7, RLENGTH - 7); exit }')"
    [ -n "${first_card}" ] && echo "plughw:${first_card},${first_device:-0}"
}

echo "=== counter-test: the index-based implementation on the same fixtures ==="
old_before="$(legacy_detect pi_before)"
old_after="$(legacy_detect pi_after)"
echo "        index-based: '${old_before}' (before) vs '${old_after}' (after)"
if [ "${old_before}" != "${old_after}" ]; then
    pass "the index-based form DID change across the reboot — the defect is real"
else
    fail "the index-based form was stable here; the fixtures no longer show the defect"
fi
if [ "${old_before}" = "plughw:1,0" ] && [ "${old_after}" = "plughw:3,0" ]; then
    pass "and it changed exactly as observed on hardware (plughw:1,0 -> plughw:3,0)"
else
    fail "index-based outputs did not match what the Pi actually produced"
fi

echo
[ "${FAILED}" = 0 ] && echo "alsa-device-detect: ALL-PASS" || echo "alsa-device-detect: FAILURES"
exit "${FAILED}"
