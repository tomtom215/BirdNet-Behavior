#!/usr/bin/env bash
# installer/test/birdnet-pi-host.sh — does installing on a BirdNET-Pi machine
# leave BirdNET-Pi's files alone? (OOB-03)
#
# On BirdNET-Pi, /etc/birdnet/birdnet.conf is a link to
# ~/BirdNET-Pi/birdnet.conf, which is this installer's own CONFIG_FILE. The
# installer took it as a kept config and ran chmod 0640 (which follows a link),
# `sed -i` (which replaces the link with a copy) and `>>` (which appends to the
# file behind it) on BirdNET-Pi's settings. These run the REAL installer's main
# (installer/test/lib/install-flow.sh) in a sandbox:
#
#   P1  the config is a link into a BirdNET-Pi folder: the install stops with
#       a message, before the binary is installed; the file behind the link has
#       the same bytes, mode and inode, and the link is still a link.
#   P2  the same, with `update` over an existing BirdNet-Behavior binary.
#   P3  BirdNET-Pi's recorder is enabled at boot (no link): the install stops.
#   P4  uninstall is never refused.
#   Counterparts:
#   C1  a plain config file and no BirdNET-Pi: the install runs to the end.
#   C2  BirdNET-Pi's unit files left behind, disabled: the install runs and
#       mentions them.
#
# What only a real host can prove: that BirdNET-Pi's real installer leaves the
# link and the wants/ links where this test puts them (read from its install
# script, not run here), and that `systemctl disable --now`, the remedy the
# message prints, is enough for the install that follows.
#
# Usage: installer/test/birdnet-pi-host.sh
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
new_sandbox() { SB="$(flow_new)"; SANDBOXES+=("${SB}"); mkdir -p "${SB}/systemd/multi-user.target.wants"; }

# A BirdNET-Pi-shaped settings file behind a link at this installer's config.
seed_birdnet_pi_link() {
    mkdir -p "${SB}/home/BirdNET-Pi"
    cat >"${SB}/home/BirdNET-Pi/birdnet.conf" <<'EOF'
LATITUDE=51.5
LONGITUDE=-0.12
MODEL=BirdNET_GLOBAL_6K_V2.4_Model_FP16
RECS_DIR=$HOME/BirdSongs
CADDY_PWD=
EOF
    chmod 0644 "${SB}/home/BirdNET-Pi/birdnet.conf"
    ln -s "${SB}/home/BirdNET-Pi/birdnet.conf" "$(flow_config "${SB}")"
}
fingerprint() { stat -c '%i %a %s' "$1"; sha256sum <"$1" | awk '{print $1}'; }

check_untouched() { # $1 = label  $2 = fingerprint before
    local target="${SB}/home/BirdNET-Pi/birdnet.conf"
    if [ "$(fingerprint "${target}")" = "$2" ]; then
        pass "$1: BirdNET-Pi's file has the same bytes, mode and inode"
    else
        fail "$1: BirdNET-Pi's file changed: was '$2', now '$(fingerprint "${target}")'"
    fi
    if [ -L "$(flow_config "${SB}")" ]; then
        pass "$1: the link is still a link"
    else
        fail "$1: the link was replaced by a file"
    fi
}

echo "=== P1 a fresh install over a BirdNET-Pi config link ==="
new_sandbox
seed_birdnet_pi_link
before="$(fingerprint "${SB}/home/BirdNET-Pi/birdnet.conf")"
flow_run "${SB}"; rc=$?
if [ "${rc}" -ne 0 ]; then pass "P1: the install stopped (exit ${rc})"; else fail "P1: the install ran to the end (exit 0)"; fi
if grep -q "BirdNET-Pi on it" "${SB}/out.log"; then pass "P1: it says why"; else fail "P1: no message naming BirdNET-Pi"; fi
if grep -q "systemctl disable --now" "${SB}/out.log"; then pass "P1: and what to do"; else fail "P1: no remedy printed"; fi
if [ ! -e "${SB}/bin/birdnet-behavior" ]; then pass "P1: nothing was installed"; else fail "P1: the binary was installed before stopping"; fi
check_untouched P1 "${before}"

echo "=== P2 an update over a BirdNET-Pi config link ==="
new_sandbox
seed_birdnet_pi_link
flow_seed_binary "${SB}"
before="$(fingerprint "${SB}/home/BirdNET-Pi/birdnet.conf")"
flow_run "${SB}" update; rc=$?
if [ "${rc}" -ne 0 ]; then pass "P2: the update stopped (exit ${rc})"; else fail "P2: the update ran to the end"; fi
check_untouched P2 "${before}"

echo "=== P3 BirdNET-Pi's recorder enabled at boot ==="
new_sandbox
: >"${SB}/systemd/birdnet_recording.service"
ln -s ../birdnet_recording.service "${SB}/systemd/multi-user.target.wants/birdnet_recording.service"
flow_run "${SB}"; rc=$?
if [ "${rc}" -ne 0 ] && grep -q "birdnet_recording.service is enabled" "${SB}/out.log"; then
    pass "P3: stopped, naming the enabled recorder"
else
    fail "P3: exit ${rc}; $(grep -c BirdNET-Pi "${SB}/out.log") lines about BirdNET-Pi"
fi
if [ ! -e "$(flow_config "${SB}")" ]; then pass "P3: no config was written"; else fail "P3: a config was written"; fi

echo "=== P4 uninstall is never refused ==="
new_sandbox
seed_birdnet_pi_link
flow_seed_binary "${SB}"
flow_run "${SB}" uninstall
if grep -q "BirdNET-Pi on it" "${SB}/out.log"; then fail "P4: uninstall was refused"; else pass "P4: uninstall was not refused"; fi

echo "=== C1 counterpart: no BirdNET-Pi ==="
new_sandbox
flow_run "${SB}"; rc=$?
if [ "${rc}" -eq 0 ] && [ -f "$(flow_config "${SB}")" ]; then
    pass "C1: installed, config written"
else
    fail "C1: exit ${rc}: $(tail -n 3 "${SB}/out.log")"
fi
if grep -qE "BirdNET-Pi on it|BirdNET-Pi was installed here" "${SB}/out.log"; then fail "C1: a BirdNET-Pi warning on a machine without it"; else pass "C1: no BirdNET-Pi warning"; fi

echo "=== C2 counterpart: BirdNET-Pi's units left behind, disabled ==="
new_sandbox
: >"${SB}/systemd/birdnet_recording.service"
: >"${SB}/systemd/birdnet_analysis.service"
flow_run "${SB}"; rc=$?
if [ "${rc}" -eq 0 ]; then pass "C2: installed"; else fail "C2: exit ${rc}: $(tail -n 3 "${SB}/out.log")"; fi
if grep -q "is disabled" "${SB}/out.log"; then pass "C2: the leftovers are mentioned"; else fail "C2: leftovers not mentioned"; fi

if [ "${FAILED}" -ne 0 ]; then
    echo "birdnet-pi-host: FAILED"
    exit 1
fi
echo "birdnet-pi-host: all pass"
