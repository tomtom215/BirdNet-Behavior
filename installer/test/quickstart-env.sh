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

echo "=== the Docker station is born with an admin password, in a file only its owner can read ==="
# The compose file publishes the dashboard on every interface and viewing is
# open, so a .env with no CADDY_PWD left /admin open to the whole LAN. The
# installer generates one; quickstart.sh did not mention a password at all.
genfn="$(extract gen_password "${QS}")"
if [ -z "${genfn}" ]; then
    fail "quickstart.sh has no gen_password"
else
    # Both branches, under the strictness the script runs with, each call in a
    # process of its own. In a `|| …` list bash switches errexit off inside a
    # command substitution, so a check written that way cannot see the failure
    # this exists to catch: the urandom branch once returned 141 from every
    # call under `set -eo pipefail` (`tr … </dev/urandom | head -c 22`; nothing
    # ever ends /dev/urandom, so `tr` always takes SIGPIPE), and the
    # second branch here passed that mutation until it was run this way.
    eval "${genfn}"     # the writer block below calls it in this shell
    nobin="$(mktemp -d)"
    for t in tr head; do ln -s "$(command -v "$t")" "${nobin}/${t}"; done
    bash_bin="$(command -v bash)"
    gen() { # $1 = PATH for the child
        env PATH="$1" "${bash_bin}" -c 'set -eo pipefail; eval "$1"; gen_password' _ "${genfn}"
    }
    for mode in openssl urandom; do
        bad=0
        for _ in $(seq 1 40); do
            if [ "${mode}" = openssl ]; then pw="$(gen "${PATH}")"; else pw="$(gen "${nobin}")"; fi
            rc=$?
            if [ "${rc}" -ne 0 ] || [ "${#pw}" -ne 22 ]; then bad=1; break; fi
            case "${pw}" in *[!A-Za-z0-9]*) bad=1; break ;; esac
        done
        if [ "${bad}" -eq 0 ]; then pass "gen_password (${mode} branch) gives 22 letters and digits, 40 of 40 under errexit and pipefail"
        else fail "gen_password (${mode} branch) failed (status ${rc}) or gave an unusable password ('${pw}')"; fi
    done
    rm -rf "${nobin}"
fi

# The writer itself: run its block in a scratch directory with a lax umask.
block="$(sed -n '/^hdr "Writing your \.env"/,/^ok "Wrote/p' "${QS}")"
if [ -z "${block}" ]; then
    fail "could not find the .env writer in quickstart.sh"
else
    scratch="$(mktemp -d)"
    (
        cd "${scratch}" || exit 1
        umask 022
        hdr() { :; }; ok() { :; }; fail() { echo "writer failed: $*" >&2; exit 1; }
        # shellcheck disable=SC2034  # read by the eval'd writer block
        {
            LAT=51.5; LON=-0.1; STATION_TZ=UTC; AUDIO_KIND=rtsp
            AUDIO_VALUE='rtsp://u:p@cam.lan/s'; IMAGE_TAG=latest
        }
        eval "${block}"
        touch after-the-writer && stat -c %a after-the-writer > mode-after
    )
    envfile="${scratch}/.env"
    if [ -f "${envfile}" ]; then
        check "$(stat -c %a "${envfile}")" "600" ".env is readable by its owner only"
        pw="$(sed -n 's/^CADDY_PWD=//p' "${envfile}")"
        if [ "${#pw}" -eq 22 ] && [ "$(grep -c '^CADDY_PWD=' "${envfile}")" -eq 1 ]; then
            pass ".env carries exactly one generated CADDY_PWD"
        else
            fail ".env has no usable CADDY_PWD (got '${pw}')"
        fi
        if grep -q "^BIRDNET_RTSP_URL=" "${envfile}"; then pass "the rest of .env is still written"
        else fail "the rest of .env went missing"; fi
        check "$(cat "${scratch}/mode-after" 2>/dev/null)" "644" "the writer did not leave its umask behind"
    else
        fail "the writer block produced no .env"
    fi
    rm -rf "${scratch}"
fi
if grep -qE '^say "  \$\{BLD\}Admin login:\$\{RST\}.*\$\{ADMIN_PASSWORD\}' "${QS}"; then
    pass "the summary shows the password once"
else
    fail "the summary does not show the generated password"
fi

echo "=== the host's timezone reaches .env, so the container does not run on UTC ==="
# shellcheck disable=SC1090
source <(extract detect_host_tz "${QS}"; extract tz_name_ok "${QS}")
if ! declare -F detect_host_tz >/dev/null; then
    fail "quickstart.sh has no detect_host_tz — nothing writes TZ, so compose's container runs on UTC"
else
    tzroot="$(mktemp -d)"
    mkdir -p "${tzroot}/a/etc" "${tzroot}/b/etc" "${tzroot}/c/etc" "${tzroot}/d/etc" "${tzroot}/e/etc"
    printf 'Europe/Berlin\n' > "${tzroot}/a/etc/timezone"
    ln -s /usr/share/zoneinfo/America/New_York "${tzroot}/b/etc/localtime"
    ln -s /var/db/timezone/zoneinfo/Australia/Sydney "${tzroot}/c/etc/localtime"
    printf '\n' > "${tzroot}/d/etc/timezone"; ln -s /etc/zz-not-a-zone "${tzroot}/d/etc/localtime"
    printf '../../etc/passwd\n' > "${tzroot}/e/etc/timezone"
    check "$(detect_host_tz "${tzroot}/a")" "Europe/Berlin"      "Debian / Raspberry Pi OS /etc/timezone"
    check "$(detect_host_tz "${tzroot}/b")" "America/New_York"   "a systemd /etc/localtime symlink"
    check "$(detect_host_tz "${tzroot}/c")" "Australia/Sydney"   "a macOS /etc/localtime symlink"
    check "$(detect_host_tz "${tzroot}/d")" ""                   "nothing usable detects nothing, not a guess"
    check "$(detect_host_tz "${tzroot}/e")" ""                   "a value that is not a zone name is refused"
    rm -rf "${tzroot}"
fi
if grep -qE "^[[:space:]]*printf 'TZ=%s\\\\n' \"\\\$STATION_TZ\"" "${QS}"; then
    pass "the .env writer writes TZ"
else
    fail "the .env writer does not write TZ"
fi

echo "=== the classifier choice reaches .env only when the operator made one ==="
# An unset choice must leave .env without a BIRDNET_CLASSIFIER line: the
# container decides (V2.4 for a new volume, V3.0 for one that already runs it),
# and a line written here would pin that decision for volumes it should not.
write_env() { # $1 = CLASSIFIER_CHOICE ("" for unset) -> prints the .env
    local scratch
    scratch="$(mktemp -d)"
    (
        cd "${scratch}" || exit 1
        hdr() { :; }; ok() { :; }; fail() { echo "writer failed: $*" >&2; exit 1; }
        # shellcheck disable=SC2034  # read by the eval'd writer block
        {
            LAT=51.5; LON=-0.1; STATION_TZ=UTC; AUDIO_KIND=none
            AUDIO_VALUE=''; IMAGE_TAG=latest
        }
        # shellcheck disable=SC2034  # read by the eval'd writer block
        if [ -n "$1" ]; then CLASSIFIER_CHOICE="$1"; else unset CLASSIFIER_CHOICE; fi
        eval "${block}"
    ) >/dev/null 2>&1
    cat "${scratch}/.env" 2>/dev/null
    rm -rf "${scratch}"
}
if [ -z "$(write_env '' | grep '^BIRDNET_CLASSIFIER=' || true)" ] && [ -n "$(write_env '' | grep '^BIRDNET_IMAGE_TAG=' || true)" ]; then
    pass "no choice: no BIRDNET_CLASSIFIER line (and the rest of .env is written)"
else
    fail "an unset choice wrote a BIRDNET_CLASSIFIER line, or no .env"
fi
check "$(write_env birdnet-v3 | grep '^BIRDNET_CLASSIFIER=' || true)" "BIRDNET_CLASSIFIER=birdnet-v3" "BIRDNET_CLASSIFIER=birdnet-v3 is written"
check "$(write_env birdnet-v24 | grep '^BIRDNET_CLASSIFIER=' || true)" "BIRDNET_CLASSIFIER=birdnet-v24" "BIRDNET_CLASSIFIER=birdnet-v24 is written"
# A typo must stop the script before anything is started, not be written for the
# entrypoint to refuse on every restart.
if grep -qE 'BIRDNET_CLASSIFIER=\$\{CLASSIFIER_CHOICE\} is not a classifier' "${QS}" \
    && grep -qE '"" \| birdnet-v24 \| birdnet-v3\) ;;' "${QS}"; then
    pass "an unknown BIRDNET_CLASSIFIER is refused by name before any prompt"
else
    fail "quickstart.sh does not validate BIRDNET_CLASSIFIER"
fi

if [ "${FAILED}" -ne 0 ]; then
    echo "quickstart-env: FAILED"
    exit 1
fi
echo "quickstart-env: all pass"
