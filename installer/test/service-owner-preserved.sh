#!/usr/bin/env bash
# installer/test/service-owner-preserved.sh — does an update from another
# account keep the station it finds?
#
# require_root sets SERVICE_USER from SUDO_USER on every run, and every
# home-based path follows it. So `sudo install.sh update` run from a second
# account rewrote the unit's User=, ReadWritePaths= and --analytics-db onto
# that account's home and chowned the config to it, while the kept config's
# DB_PATH still named the first home — which the unit's ProtectHome hides
# from the new user. The doctor failed the database directory, and the
# station did not start.
#
# Usage: installer/test/service-owner-preserved.sh
# Needs bash + coreutils. No root.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
FAILED=0
pass() { printf '  PASS  %s\n' "$*"; }
fail() { printf '  FAIL  %s\n' "$*"; FAILED=1; }

WORK="$(mktemp -d)"
trap 'rm -rf "${WORK}"' EXIT

# Run the shipping functions against a unit that names $1, with the installer
# invoked by $2. Prints the resolved owner and data dir.
resolve() { # $1=unit user (empty = no unit)  $2=sudo user
    rm -f "${WORK}/unit.service"
    if [ -n "$1" ]; then printf '[Service]\nUser=%s\n' "$1" > "${WORK}/unit.service"; fi
    (
        set -euo pipefail
        # shellcheck disable=SC1090
        source <(sed -n '/^derive_home_paths()/,/^}/p' "${REPO_ROOT}/installer/lib/30-platform.sh")
        # shellcheck disable=SC1090
        source <(sed -n '/^adopt_existing_service_user()/,/^}/p' "${REPO_ROOT}/installer/lib/45-preflight.sh")
        warn() { :; }
        getent() { # passwd <user>
            case "$2" in
                alice) echo "alice:x:1001:1001::/home/alice:/bin/bash" ;;
                bob)   echo "bob:x:1002:1002::/home/bob:/bin/bash" ;;
                *)     return 2 ;;
            esac
        }
        # shellcheck disable=SC2034  # read by the sourced function
        SERVICE_FILE="${WORK}/unit.service"
        SERVICE_USER="$2"
        derive_home_paths
        adopt_existing_service_user
        printf '%s %s\n' "${SERVICE_USER}" "${DATA_DIR}"
    )
}

check() { if [ "$1" = "$2" ]; then pass "$3"; else fail "$3 — got '$1', wanted '$2'"; fi; }

echo "=== an update from another account keeps the station's owner ==="
check "$(resolve alice bob)" "alice /home/alice/BirdNet-Behavior" \
    "the unit's user and home win over SUDO_USER"

echo "=== counterparts ==="
check "$(resolve bob bob)" "bob /home/bob/BirdNet-Behavior" "the same account changes nothing"
check "$(resolve "" bob)" "bob /home/bob/BirdNet-Behavior" "a fresh install uses SUDO_USER"
check "$(resolve ghost bob)" "bob /home/bob/BirdNet-Behavior" \
    "a unit whose user no longer exists falls back to SUDO_USER"

echo "=== main runs it before anything is written ==="
if awk '/require_root/{r=NR} /detect_existing_install/{d=NR} /adopt_existing_service_user/{a=NR} END{exit !(r && d && a && r<d && d<a)}' \
    "${REPO_ROOT}/installer/lib/95-main.sh"; then
    pass "main calls adopt_existing_service_user after detect_existing_install"
else
    fail "main never adopts the existing unit's owner"
fi

if [ "${FAILED}" -ne 0 ]; then
    echo "service-owner-preserved: FAILED"
    exit 1
fi
echo "service-owner-preserved: all pass"
