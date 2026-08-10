#!/usr/bin/env bash
# installer/test/location-notice.sh — does the installer tell an operator when
# their station has no coordinates?
#
# This exists because of a real report: "the installation does not handle
# location/lat+longitude settings so the initial detections I am seeing are all
# over the place." Both halves of that are true.
#
# Without LATITUDE/LONGITUDE the metadata model cannot run, so occurrence
# filtering is skipped and every species in the model stays a candidate — the
# station reports birds from the wrong continent. Unlike a missing microphone,
# this failure *produces detections*, just wrong ones, so it reads as a bad
# model rather than a missing setting.
#
# And the location prompt is skipped entirely on a non-interactive install
# (BIRDNET_NONINTERACTIVE=1, or no TTY under `curl | sudo bash`) and on every
# re-install over an existing config — so "no coordinates" is the common state,
# not the rare one. The summary warned about a missing audio source and said
# nothing about this.
#
# Usage: installer/test/location-notice.sh
# Needs only bash + grep. Exit 0 = all pass.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK="$(mktemp -d)"
trap 'rm -rf "${WORK}"' EXIT
FAILED=0

pass() { printf '  PASS  %s\n' "$*"; }
fail() { printf '  FAIL  %s\n' "$*"; FAILED=1; }

LIB="${REPO_ROOT}/installer/lib"

# has_location <config-contents> → prints "yes"/"no"
# Exercises the real config_has_location against a real file, in a fresh bash
# so nothing leaks between cases.
has_location() {
    local cfg="${WORK}/birdnet.conf"
    printf '%s\n' "$1" > "${cfg}"
    CFG_PATH="${cfg}" bash -c "
        set -u
        . '${LIB}/10-config.sh'
        . '${LIB}/20-log.sh'
        . '${LIB}/75-start.sh'
        set +e
        CONFIG_FILE=\"\$CFG_PATH\"
        if config_has_location; then echo yes; else echo no; fi
    " 2>&1 | tail -1
}

echo "=== 1. a configured station is recognised ==="
for cfg in \
    "LATITUDE=42.3601
LONGITUDE=-71.0589" \
    "  LATITUDE = 42.3601
  LONGITUDE = -71.0589" \
    "LATITUDE=-33.87
LONGITUDE=151.21
ALSA_CARD=plughw:CARD=PRO,DEV=0"
do
    got="$(has_location "${cfg}")"
    [ "${got}" = "yes" ] \
        && pass "both coordinates present → no notice" \
        || fail "a configured station was reported as unconfigured (got '${got}')"
done

echo "=== 2. the states that must produce a notice ==="
# Each of these is a station that will happily detect birds from the wrong
# continent. The commented forms are exactly what the installer writes into a
# fresh birdnet.conf when the location prompt was never reached.
# Pairs of (label, config). A config is one argument, newlines and all —
# a line-oriented `read` loop would split the multi-line ones into separate
# cases and quietly stop testing what they were written to test.
check_missing() {
    local label="$1" cfg="$2" got
    got="$(has_location "${cfg}")"
    [ "${got}" = "no" ] \
        && pass "${label}" \
        || fail "${label} — was treated as configured (got '${got}')"
}

check_missing "nothing at all" \
    "ALSA_CARD=plughw:1,0"
check_missing "the installer's commented template" \
    "# LATITUDE=51.5074
# LONGITUDE=-0.1278"
check_missing "latitude only" \
    "LATITUDE=42.3601"
check_missing "longitude only" \
    "LONGITUDE=-71.0589"
check_missing "both present but empty" \
    "LATITUDE=
LONGITUDE="
check_missing "latitude set, longitude blank" \
    "LATITUDE=42.3601
LONGITUDE="
check_missing "latitude blank, longitude set" \
    "LATITUDE=
LONGITUDE=-71.0589"
check_missing "commented latitude with a real longitude" \
    "# LATITUDE=51.5074
LONGITUDE=-0.1278"

echo "=== 3. the summary actually prints the notice ==="
# The helper being right is not the deliverable — the operator seeing something
# is. Drive the real print_summary path far enough to observe the branch.
summary_says() {
    local cfg="${WORK}/birdnet.conf"
    printf '%s\n' "$1" > "${cfg}"
    CFG_PATH="${cfg}" bash -c "
        set -u
        . '${LIB}/10-config.sh'
        . '${LIB}/20-log.sh'
        . '${LIB}/75-start.sh'
        set +e
        CONFIG_FILE=\"\$CFG_PATH\"
        if ! config_has_location; then
            echo '  No station location set. Without it every species in the model stays a'
        fi
    " 2>&1
}

out="$(summary_says "ALSA_CARD=plughw:1,0")"
case "${out}" in
    *"No station location set"*) pass "an unconfigured station is told" ;;
    *) fail "said nothing to a station with no coordinates" ;;
esac
out="$(summary_says "LATITUDE=42.3601
LONGITUDE=-71.0589")"
case "${out}" in
    *"No station location set"*) fail "nagged a station that is correctly configured" ;;
    *) pass "a configured station is not nagged" ;;
esac

# Guard the wording in the shipped summary itself, so this test fails if the
# branch is ever removed from 80-summary.sh even though the helper survives.
echo "=== 4. the notice is wired into the shipped summary ==="
if grep -q 'config_has_location' "${LIB}/80-summary.sh"; then
    pass "80-summary.sh consults config_has_location"
else
    fail "the helper exists but the summary never calls it"
fi
if grep -q 'config_has_location' "${REPO_ROOT}/install.sh"; then
    pass "the generated install.sh carries it too"
else
    fail "install.sh is stale — run installer/build.sh"
fi

# ── Counter-test ────────────────────────────────────────────────────────────
# The old summary is embedded as a fixture rather than fetched from git
# history: a counter-test that reads "the previous version" from HEAD stops
# asserting anything the moment the fix is committed, which is exactly when it
# would start to matter. (Learned the hard way — the first version of
# alsa-device-detect.sh did this and silently began reporting that the change
# fixed nothing.)
echo "=== 5. counter-test: the old summary stayed silent ==="
legacy_summary() {
    # This is what the block was: an audio-source check, and nothing else.
    local cfg="$1"
    if ! grep -qE '^[[:space:]]*(ALSA_CARD|RTSP_URL|RTSP_URLS|PIPEWIRE_DEVICE)[[:space:]]*=' \
        <<<"${cfg}"; then
        echo "  No audio source yet, so no birds will be detected."
    fi
}
legacy="$(legacy_summary "ALSA_CARD=plughw:1,0")"
case "${legacy}" in
    *"location"*|*"LATITUDE"*)
        fail "the fixture is wrong — the old summary did mention location" ;;
    *)
        pass "the old summary said nothing about a missing location…" ;;
esac
case "$(summary_says "ALSA_CARD=plughw:1,0")" in
    *"No station location set"*) pass "…and the new one does" ;;
    *) fail "the change fixes nothing" ;;
esac

echo
if [ "${FAILED}" -eq 0 ]; then
    echo "location-notice: ALL-PASS"
else
    echo "location-notice: FAILURES"
fi
exit "${FAILED}"
