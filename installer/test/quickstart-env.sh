#!/usr/bin/env bash
# installer/test/quickstart-env.sh — does quickstart.sh write a .env the
# container can start from, holding what the operator typed?
#
# quickstart.sh wrote the answers into .env as typed. Two ways that broke a
# station, both confirmed before this test was written:
#
#   * A latitude with a decimal comma, or the "lat, lon" pair a map hands you,
#     went in as BIRDNET_LATITUDE=42,36. The daemon's argument parser rejects
#     it ("invalid value '42,36' for '--latitude'", exit 2) before logging a
#     line, and `restart: unless-stopped` restarts it into the same error.
#   * The RTSP URL went in unquoted. Compose interpolates `$` in .env values, so
#     `docker compose config` turned rtsp://u:pa$word@h/s into rtsp://u:pa@h/s —
#     a camera password quietly cut short. Single quotes are literal.
#
# The installer already had a parser for exactly the coordinate inputs
# (installer/lib/70-station.sh parse_coords); quickstart.sh is downloaded on
# its own, so it carries a copy, and this test holds the copy identical.
#
# Usage: installer/test/quickstart-env.sh
# Needs bash + awk; the Compose round trip runs when `docker compose` exists.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
QS="${REPO_ROOT}/quickstart.sh"
FAILED=0
pass() { printf '  PASS  %s\n' "$*"; }
fail() { printf '  FAIL  %s\n' "$*"; FAILED=1; }

extract() { sed -n "/^$1()/,/^}/p" "$2"; }

echo "=== quickstart.sh's parse_coords is the installer's ==="
installer_copy="$(extract parse_coords "${REPO_ROOT}/installer/lib/70-station.sh")"
quickstart_copy="$(extract parse_coords "${QS}")"
if [ -n "${quickstart_copy}" ] && [ "${quickstart_copy}" = "${installer_copy}" ]; then
    pass "the two copies are identical"
else
    fail "quickstart.sh has no parse_coords, or it differs from installer/lib/70-station.sh"
fi

# shellcheck disable=SC1090
source <(extract parse_coords "${QS}"; extract take_coords "${QS}"; \
         extract take_longitude "${QS}"; extract env_quote "${QS}")

coords() { # $1=input  -> "LAT|LON|status"
    LAT=""; LON=""
    if take_coords "$1" 2>/dev/null; then echo "${LAT}|${LON}|ok"; else echo "${LAT}|${LON}|refused"; fi
}
check() { # $1=got  $2=want  $3=what
    if [ "$1" = "$2" ]; then pass "$3"; else fail "$3 — got '$1', wanted '$2'"; fi
}

echo "=== what an operator types becomes a number the daemon accepts ==="
check "$(coords '42.3601')"           "42.3601||ok"         "a dotted latitude"
check "$(coords '49,4521')"           "49.4521||ok"         "a decimal comma"
check "$(coords '42.36, -71.05')"     "42.36|-71.05|ok"     "a pair pasted from a map"
check "$(coords '49,4521, 8,6724')"   "49.4521|8.6724|ok"   "a pair with decimal commas"
check "$(coords 'abc')"               "||refused"           "garbage is refused"
check "$(coords '95')"                "||refused"           "a latitude out of range is refused"
LON=""; take_longitude '-71,0589' 2>/dev/null
check "${LON}" "-71.0589" "a longitude with a decimal comma"
LON=""; if take_longitude '200' 2>/dev/null; then r=ok; else r=refused; fi
check "${r}" "refused" "a longitude out of range is refused"

echo "=== a value with \$ reaches the container intact ==="
url='rtsp://user:pa$word@camera.lan:554/stream'
line="BIRDNET_RTSP_URL=$(env_quote "${url}")"
check "${line}" "BIRDNET_RTSP_URL='${url}'" "the RTSP URL is single-quoted"
if env_quote "it's" >/dev/null 2>&1; then
    fail "a value holding ' was quoted anyway — it would end the quoting early"
else
    pass "a value holding ' is refused"
fi
if docker compose version >/dev/null 2>&1; then
    work="$(mktemp -d)"
    printf 'services:\n  s:\n    image: busybox\n    environment:\n      URL: ${BIRDNET_RTSP_URL}\n' \
        > "${work}/docker-compose.yml"
    printf '%s\n' "${line}" > "${work}/.env"
    got="$(cd "${work}" && docker compose config 2>/dev/null | awk '/URL:/{print $2}' | sed 's/\$\$/$/g')"
    rm -rf "${work}"
    check "${got}" "${url}" "docker compose config keeps the password's \$"
else
    printf '  SKIP  docker compose not available — the Compose round trip was not run\n'
fi

echo "=== the .env writer uses them ==="
if grep -qF 'BIRDNET_RTSP_URL=%s\n'"'"'        "$(env_quote "$AUDIO_VALUE")"' "${QS}"; then
    pass "BIRDNET_RTSP_URL is written through env_quote"
else
    fail "BIRDNET_RTSP_URL is written without env_quote"
fi
if grep -qE "^[[:space:]]*printf 'BIRDNET_LATITUDE=%s" "${QS}" \
   && grep -qE 'if \[ -n "\$LAT" \] && \[ -n "\$LON" \]; then' "${QS}"; then
    pass "coordinates are written only when both parsed"
else
    fail "the writer does not guard the coordinates"
fi

if [ "${FAILED}" -ne 0 ]; then
    echo "quickstart-env: FAILED"
    exit 1
fi
echo "quickstart-env: all pass"
