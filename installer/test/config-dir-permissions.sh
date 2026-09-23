#!/usr/bin/env bash
# installer/test/config-dir-permissions.sh — can the service keep its last-good
# configuration, and only that?
#
# The station keeps a copy of the last configuration it started on as
# `birdnet.conf.last-good`, written beside the file via a `.part` and a rename,
# and runs on it when a later edit has errors (src/helpers/startup_config.rs).
# The installer created /etc/birdnet as root 0755 and runs the service as a
# non-root user, so the copy could never be written: every start logged "could
# not keep a last-good copy", and the rollback the config header, --apply-config
# and hardening.md all promise had nothing to roll back to.
#
# 0770 would let the service write it, and would also let the service replace
# or delete birdnet.conf itself. 1770 (sticky) lets it create and replace only
# files it owns. Measured with two real users (root-only half below):
#
#   0755: last-good cannot be created
#   0770: last-good written; birdnet.conf replaced AND deleted by the service
#   1770: last-good written twice; birdnet.conf neither replaced nor deleted
#
# Usage: installer/test/config-dir-permissions.sh
# The structural half needs bash + coreutils. The behavioural half needs root
# and useradd, and says plainly when it is skipped.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
FAILED=0
pass() { printf '  PASS  %s\n' "$*"; }
fail() { printf '  FAIL  %s\n' "$*"; FAILED=1; }

# ── Structural: what the real create_directories asks `install` for ─────────
CALLS="$(
    set -euo pipefail
    # shellcheck disable=SC1090
    source <(sed -n '/^create_directories()/,/^}/p' "${REPO_ROOT}/installer/lib/60-dirs.sh")
    info() { :; }
    success() { :; }
    install() { printf '%s\n' "$*"; }
    # shellcheck disable=SC2034  # read by the sourced function
    SERVICE_USER="birdsvc"
    DATA_DIR=/d RECS_DIR=/d/r IMAGE_CACHE_DIR=/d/i MODEL_DIR=/d/m CONFIG_DIR=/etc/birdnet
    create_directories
)"
line="$(printf '%s\n' "${CALLS}" | grep -E '(^| )/etc/birdnet$' || true)"
if [ -z "${line}" ]; then
    fail "create_directories never creates the config dir"
else
    case " ${line} " in
        *" -m 1770 "*) pass "config dir mode is 1770 (${line})" ;;
        *) fail "config dir is not 1770 — the service cannot keep a last-good copy safely (${line})" ;;
    esac
    case " ${line} " in
        *" -g birdsvc "*) pass "config dir group is the service user's" ;;
        *) fail "config dir group is not the service user's (${line})" ;;
    esac
    case " ${line} " in
        *" -o root "*) pass "config dir stays owned by root" ;;
        *) fail "config dir is not owned by root (${line})" ;;
    esac
fi

# ── Behavioural: two real users (root only) ──────────────────────────────────
if [ "$(id -u)" -ne 0 ] || ! command -v useradd >/dev/null 2>&1; then
    printf '  SKIP  behavioural half: needs root and useradd (structural half ran)\n'
else
    user="bnbcfgtest$$"
    useradd -M -s /bin/sh "${user}"
    work="$(mktemp -d /tmp/bnb-cfgdir.XXXXXX)"
    chmod 0755 "${work}"
    trap 'userdel "${user}" 2>/dev/null; rm -rf "${work}"' EXIT
    d="${work}/birdnet"
    # The mode the real create_directories asked for above, not a copy of it.
    mode="$(printf '%s\n' "${line}" | sed -n 's/.*-m \([0-7]*\).*/\1/p')"
    install -d -m "${mode:-0755}" -o root -g "${user}" "${d}"
    printf 'LATITUDE=1\n' > "${d}/birdnet.conf"
    chown "root:${user}" "${d}/birdnet.conf"
    chmod 0640 "${d}/birdnet.conf"
    as() { su -s /bin/sh "${user}" -c "$1" >/dev/null 2>&1; }
    for n in 1 2; do
        if as "cat '${d}/birdnet.conf' > '${d}/birdnet.conf.last-good.part' && mv '${d}/birdnet.conf.last-good.part' '${d}/birdnet.conf.last-good'"; then
            pass "service wrote its last-good copy (write ${n})"
        else
            fail "service could not write its last-good copy (write ${n})"
        fi
    done
    as "echo x > '${d}/x' && mv -f '${d}/x' '${d}/birdnet.conf'"
    as "rm -f '${d}/birdnet.conf'"
    if [ "$(cat "${d}/birdnet.conf" 2>/dev/null)" = "LATITUDE=1" ]; then
        pass "service could neither replace nor delete birdnet.conf"
    else
        fail "service replaced or deleted birdnet.conf"
    fi
fi

exit "${FAILED}"
