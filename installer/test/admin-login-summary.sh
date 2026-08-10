#!/usr/bin/env bash
# installer/test/admin-login-summary.sh — what the installer tells an operator
# about signing in to /admin, in each state a real station can be in.
#
# This exists because the interesting case is the one that printed nothing.
# GENERATED_ADMIN_PASSWORD and CADDY_PWD_VALUE are set only during onboarding,
# which is skipped when a config already exists — so on every upgrade and
# repair the admin-login block was silent. Harmless while /admin was served
# without a password; a lockout the moment that hole is closed, because the
# operator needs a credential they were shown once at install and has no reason
# to know where it lives.
#
# Usage: installer/test/admin-login-summary.sh
# Needs only bash + grep. Exit 0 = all pass.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK="$(mktemp -d)"
trap 'rm -rf "${WORK}"' EXIT
FAILED=0

pass() { printf '  PASS  %s\n' "$*"; }
fail() { printf '  FAIL  %s\n' "$*"; FAILED=1; }

# run_case <lib-dir> <config-contents|--none> <listen> <generated> <entered>
# Prints what the operator would see. A fresh bash each time so state cannot
# leak between cases.
run_case() {
    local libdir="$1" config="$2" listen="$3" generated="$4" entered="$5"
    local cfg="${WORK}/birdnet.conf"
    if [ "${config}" = "--none" ]; then
        rm -f "${cfg}"
    else
        printf '%s\n' "${config}" > "${cfg}"
    fi
    CFG_PATH="${cfg}" LISTEN="${listen}" GEN="${generated}" ENT="${entered}" \
    bash -c "
        set -u
        . '${libdir}/10-config.sh'
        . '${libdir}/20-log.sh'
        . '${libdir}/80-summary.sh'
        set +e
        CONFIG_FILE=\"\$CFG_PATH\"
        LISTEN_ADDR=\"\$LISTEN\"
        GENERATED_ADMIN_PASSWORD=\"\$GEN\"
        CADDY_PWD_VALUE=\"\$ENT\"
        print_admin_login
    " 2>&1
}

LIB="${REPO_ROOT}/installer/lib"

echo "=== 1. fresh install, password generated ==="
out="$(run_case "${LIB}" "CADDY_PWD=s3cret" "0.0.0.0:8502" "s3cret" "s3cret")"
printf '%s\n' "${out}" | sed 's/^/        /'
case "${out}" in
    *"username:"*admin*) pass "names the username" ;;
    *) fail "no username" ;;
esac
case "${out}" in *s3cret*) pass "shows the generated password once" ;; *) fail "password not shown" ;; esac

echo "=== 2. UPGRADE: password already in the config, none generated this run ==="
out="$(run_case "${LIB}" "CADDY_PWD=s3cret" "0.0.0.0:8502" "" "")"
printf '%s\n' "${out}" | sed 's/^/        /'
[ -n "${out//[[:space:]]/}" ] && pass "says something at all (it used to say nothing)" \
                             || fail "printed nothing — the lockout is still here"
case "${out}" in *admin*) pass "names the username" ;; *) fail "no username" ;; esac
case "${out}" in
    *"grep '^CADDY_PWD'"*) pass "gives the exact command to reveal it" ;;
    *) fail "no reveal command" ;;
esac
case "${out}" in
    *s3cret*) fail "leaked the password into upgrade scrollback" ;;
    *) pass "does not reprint the secret" ;;
esac
case "${out}" in
    *"earlier versions"*) pass "explains that sign-in is newly required" ;;
    *) fail "no mention of the behaviour change" ;;
esac

echo "=== 3. no password, dashboard on the LAN ==="
out="$(run_case "${LIB}" "ALSA_CARD=plughw:1,0" "0.0.0.0:8502" "" "")"
printf '%s\n' "${out}" | sed 's/^/        /'
case "${out}" in
    *"NO PASSWORD"*) pass "warns loudly that /admin is open" ;;
    *) fail "no warning for an unprotected LAN-reachable panel" ;;
esac

echo "=== 4. no password, bound to this device only ==="
out="$(run_case "${LIB}" "ALSA_CARD=plughw:1,0" "127.0.0.1:8502" "" "")"
printf '%s\n' "${out}" | sed 's/^/        /'
case "${out}" in
    *"NO PASSWORD"*) fail "loud warning is wrong for a loopback bind" ;;
    *"this device only"*) pass "explains the panel is local-only" ;;
    *) fail "said nothing useful" ;;
esac

echo "=== 5. commented-out CADDY_PWD is not a password ==="
out="$(run_case "${LIB}" "# CADDY_PWD=change-me" "0.0.0.0:8502" "" "")"
case "${out}" in
    *"NO PASSWORD"*) pass "a commented template does not count as set" ;;
    *) fail "treated a commented line as a configured password" ;;
esac

# ── Counter-test ────────────────────────────────────────────────────────────
# Case 2 is the whole point, so the old behaviour is kept here as a fixture and
# must fail it. Embedded rather than read from git history: a counter-test that
# fetches "the previous version" from HEAD stops asserting anything the moment
# the fix is committed, which is precisely when it would start to matter.
#
# This is the block as it stood — the entire admin section gated on two
# variables that onboarding alone ever sets.
legacy_admin_login() {
    local generated="$1" entered="$2"
    if [ -n "${generated}" ]; then
        echo "  Admin panel login: admin / ${generated}"
    elif [ -n "${entered}" ]; then
        echo "  Admin panel: sign in as 'admin' with the password you set."
    fi
}

echo "=== counter-test: the previous behaviour on case 2 (an upgrade) ==="
legacy_out="$(legacy_admin_login "" "")"
if [ -z "${legacy_out//[[:space:]]/}" ]; then
    pass "previously printed NOTHING on an upgrade — the lockout was real"
else
    fail "previous behaviour already said something; this change fixes nothing"
fi
# And it did print on a fresh install, so the defect was specific to upgrades
# rather than the block never working at all.
legacy_fresh="$(legacy_admin_login "s3cret" "s3cret")"
case "${legacy_fresh}" in
    *s3cret*) pass "...while a fresh install printed fine — upgrades only" ;;
    *) fail "fixture does not reproduce the old fresh-install behaviour" ;;
esac

echo
[ "${FAILED}" = 0 ] && echo "admin-login-summary: ALL-PASS" || echo "admin-login-summary: FAILURES"
exit "${FAILED}"
