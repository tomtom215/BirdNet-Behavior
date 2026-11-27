#!/usr/bin/env bash
# installer/test/reset-password.sh — does `install.sh reset-password` give a
# locked-out owner a password that works, and change nothing else? (OOB-07,
# N25-14)
#
# The only way back used to be `sudo grep '^CADDY_PWD'`, which prints the
# password the installer wrote: after a change on the Accounts page that is a
# stale one the station ignores. The station applies a CADDY_PWD that differs
# from the one it last applied (src/helpers/auth.rs,
# editing_caddy_pwd_replaces_a_password_set_on_the_accounts_page), so a new
# random value plus a restart is a reset. These run the REAL installer's main
# (installer/test/lib/install-flow.sh) in a sandbox:
#
#   R1  a config with CADDY_PWD=old: exit 0; exactly one active CADDY_PWD, new,
#       22 characters, the one printed; every other line byte-identical; mode
#       kept; the commented example left alone.
#   R2  a config with no CADDY_PWD line: one is added, and printed.
#   R3  no config: refused, nothing written.
#   R4  a config that is a link (BirdNET-Pi's): refused, the file behind it
#       unchanged.
#
# What only a real host can prove: that the restart this runs (systemctl,
# stubbed here) brings the station up with the new password in force.
#
# Usage: installer/test/reset-password.sh
# Needs bash + coreutils. No root, no network.

set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=installer/test/lib/install-flow.sh
source "${HERE}/lib/install-flow.sh"

FAILED=0
pass() { printf '  PASS  %s\n' "$*"; }
fail() { printf '  FAIL  %s\n' "$*"; FAILED=1; }
SANDBOXES=()
trap 'rm -rf "${SANDBOXES[@]}"' EXIT
new_sandbox() { SB="$(flow_new)"; SANDBOXES+=("${SB}"); }

active_pwds() { grep -cE '^[[:space:]]*CADDY_PWD[[:space:]]*=' "$(flow_config "${SB}")"; }
printed_pwd() { sed -n 's/.*password:  \(\x1b\[[0-9;]*m\)\{0,1\}\([A-Za-z0-9]\{22\}\).*/\2/p' "${SB}/out.log" | tail -n 1; }

echo "=== R1 a station with a password ==="
new_sandbox
cat >"$(flow_config "${SB}")" <<'EOF'
# --- Admin ---
# CADDY_PWD=change-me-to-a-strong-password
CADDY_USER=admin
CADDY_PWD=the-old-password-from-install
LATITUDE=51.5
EOF
chmod 0640 "$(flow_config "${SB}")"
others_before="$(grep -vE '^[[:space:]]*CADDY_PWD[[:space:]]*=' "$(flow_config "${SB}")")"
flow_run "${SB}" reset-password; rc=$?
if [ "${rc}" -eq 0 ]; then pass "R1: exit 0"; else fail "R1: exit ${rc}: $(tail -n 3 "${SB}/out.log")"; fi
if [ "$(active_pwds)" = 1 ]; then pass "R1: one active CADDY_PWD"; else fail "R1: $(active_pwds) active CADDY_PWD lines"; fi
new="$(flow_conf "${SB}" CADDY_PWD)"
if [ "${#new}" -eq 22 ] && [ "${new}" != "the-old-password-from-install" ]; then
    pass "R1: a new 22-character password"
else
    fail "R1: CADDY_PWD is '${new}'"
fi
if [ "$(printed_pwd)" = "${new}" ]; then pass "R1: the printed password is the one written"; else fail "R1: printed '$(printed_pwd)', wrote '${new}'"; fi
if [ "$(grep -vE '^[[:space:]]*CADDY_PWD[[:space:]]*=' "$(flow_config "${SB}")")" = "${others_before}" ]; then
    pass "R1: every other line is unchanged, the commented example included"
else
    fail "R1: other lines changed"
fi
if [ "$(stat -c %a "$(flow_config "${SB}")")" = 640 ]; then pass "R1: mode kept"; else fail "R1: mode is $(stat -c %a "$(flow_config "${SB}")")"; fi
if ls "${SB}/etc/birdnet/" | grep -q reset; then fail "R1: a temporary file was left behind"; else pass "R1: no temporary file left"; fi

echo "=== R2 a station with no CADDY_PWD ==="
new_sandbox
printf 'LATITUDE=51.5\n' >"$(flow_config "${SB}")"
flow_run "${SB}" reset-password; rc=$?
new="$(flow_conf "${SB}" CADDY_PWD)"
if [ "${rc}" -eq 0 ] && [ "${#new}" -eq 22 ] && [ "$(printed_pwd)" = "${new}" ]; then
    pass "R2: a password added and printed"
else
    fail "R2: exit ${rc}, CADDY_PWD '${new}'"
fi
if grep -qx 'LATITUDE=51.5' "$(flow_config "${SB}")"; then pass "R2: the rest kept"; else fail "R2: LATITUDE lost"; fi

echo "=== R3 no station ==="
new_sandbox
flow_run "${SB}" reset-password; rc=$?
if [ "${rc}" -ne 0 ] && [ ! -e "$(flow_config "${SB}")" ]; then pass "R3: refused, nothing written"; else fail "R3: exit ${rc}"; fi

echo "=== R4 a config that is BirdNET-Pi's link ==="
new_sandbox
mkdir -p "${SB}/home/BirdNET-Pi"
printf 'CADDY_PWD=birdnet-pi-own\n' >"${SB}/home/BirdNET-Pi/birdnet.conf"
ln -s "${SB}/home/BirdNET-Pi/birdnet.conf" "$(flow_config "${SB}")"
flow_run "${SB}" reset-password; rc=$?
if [ "${rc}" -ne 0 ] && [ "$(cat "${SB}/home/BirdNET-Pi/birdnet.conf")" = "CADDY_PWD=birdnet-pi-own" ] && [ -L "$(flow_config "${SB}")" ]; then
    pass "R4: refused, BirdNET-Pi's file unchanged"
else
    fail "R4: exit ${rc}"
fi

if [ "${FAILED}" -ne 0 ]; then
    echo "reset-password: FAILED"
    exit 1
fi
echo "reset-password: all pass"
