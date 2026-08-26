#!/usr/bin/env bash
# installer/test/pipefail-sigpipe.sh — the installer runs under `set -euo
# pipefail`, so a pipeline whose consumer stops reading early is a bug.
#
# # The mechanism
#
# `producer | consumer` where the consumer exits at the first match or the
# first N bytes — `grep -q`, `head -1`, `head -c` — leaves the producer writing
# into a closed pipe. The producer takes SIGPIPE and dies with status 141.
# `set -o pipefail` promotes that to the pipeline's status, and then:
#
#   * in an assignment (`x="$(a | head -1)"`), `set -e` kills the script —
#     silently, with no message on any stream;
#   * in an `if`, the condition reads false even though the consumer matched.
#
# # What it had actually broken here
#
# Measured against the code as it stood, not reasoned about:
#
#   installer/lib/70-station.sh  gen_password's no-openssl fallback,
#       `tr -dc … </dev/urandom | head -c 22`. /dev/urandom never ends, so the
#       producer is *always* mid-write: 200 aborts in 200 runs. The caller
#       assigns the result, so on any system without openssl the installer died
#       outright at the step that secures /admin.
#   installer/lib/30-platform.sh  `ldd --version | head -1 | …` in an
#       assignment: 3 aborts in 200 runs. Reached only when
#       `getconf GNU_LIBC_VERSION` is empty, which is why Debian never saw it.
#   installer/lib/50-binary.sh  `find … | head -1` in an assignment:
#       deterministic with several matches, clean with one.
#   installer/lib/76-validate.sh, 77-manage.sh  `producer | grep -q` in an
#       `if`: 1 wrong answer in 300 with a 5000-line producer.
#
# # The rule
#
# Do not pipe into an early-exiting consumer. `awk 'NR==1'` instead of
# `head -1` (awk reads to the end), and capture-then-match against a
# here-string instead of `producer | grep -q` (a here-string has no producer
# process to signal). `grep -q FILE` and `grep -q … <<<"$var"` are fine and are
# not flagged.
#
# Usage: installer/test/pipefail-sigpipe.sh
# Needs bash + coreutils. Exit 0 = all pass.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
FAILED=0

pass() { printf '  PASS  %s\n' "$*"; }
fail() { printf '  FAIL  %s\n' "$*"; FAILED=1; }

echo "=== 1. the mechanism still reproduces on this system ==="
# If it ever stops, the rule below is obsolete and should be revisited rather
# than cargo-culted. Both of these were measured deterministic (100/100 and
# 50/50) before being asserted here.
if bash -c 'set -euo pipefail; seq 1 2000000 | grep -q "^1$"' 2>/dev/null; then
    fail "a large producer piped into 'grep -q' no longer reports failure — \
the SIGPIPE/pipefail interaction has changed and this file needs revisiting"
else
    pass "'producer | grep -q' reports failure under pipefail despite matching"
fi
if bash -c 'set -euo pipefail; LC_ALL=C tr -dc "A-Za-z0-9" </dev/urandom | head -c 22 >/dev/null' 2>/dev/null; then
    fail "'tr </dev/urandom | head -c' no longer reports failure — revisit this file"
else
    pass "'endless producer | head -c' reports failure under pipefail"
fi

echo
echo "=== 2. no shell source pipes into an early-exiting consumer ==="
# Comments are stripped first so the explanations above (which name the very
# constructs being banned) do not trip the lint. The leading `[^|]` is what
# distinguishes a pipe from a logical OR: `a || grep -q x FILE` is not a
# pipeline and has no producer to signal, and matching it was a false positive
# on two correct lines.
SOURCES=("${REPO_ROOT}"/installer/lib/*.sh "${REPO_ROOT}/uninstall.sh" "${REPO_ROOT}/quickstart.sh")
offenders=0
for f in "${SOURCES[@]}"; do
    [ -f "${f}" ] || continue
    while IFS= read -r hit; do
        printf '  FAIL  %s: %s\n' "${f#"${REPO_ROOT}/"}" "${hit}"
        offenders=$((offenders + 1))
        FAILED=1
    done < <(
        sed -e 's/[[:space:]]*#.*$//' "${f}" \
            | grep -nE '[^|]\|[[:space:]]*(head[[:space:]]+-|grep[[:space:]]+-[a-zA-Z]*q[a-zA-Z]*([[:space:]]|$))' \
            || true
    )
done
if [ "${offenders}" -eq 0 ]; then
    pass "no '| head -N' or '| grep -q' in $(printf '%s\n' "${SOURCES[@]}" | wc -l) shell sources"
else
    printf '        Use %s instead of %s, and capture into a variable then\n' \
        "awk 'NR==1'" "head -1"
    printf '        match with grep -q <<<\"\$var\" instead of piping into grep -q.\n'
fi

echo
echo "=== 3. counterpart: the lint above can actually find something ==="
# Otherwise "no offenders" is equally consistent with a regex that matches
# nothing at all.
PROBE="$(mktemp)"
cat >"${PROBE}" <<'PEOF'
#!/usr/bin/env bash
x="$(find / -name foo | head -1)"
if lsmod | grep -q '^zram'; then :; fi
# this line is a comment mentioning | head -1 and must NOT be flagged
grep -q pattern /etc/hosts
[ -z "$x" ] || grep -q pattern /etc/hosts
grep -q pattern <<<"$x"
PEOF
found="$(sed -e 's/[[:space:]]*#.*$//' "${PROBE}" \
    | grep -cE '[^|]\|[[:space:]]*(head[[:space:]]+-|grep[[:space:]]+-[a-zA-Z]*q[a-zA-Z]*([[:space:]]|$))' || true)"
rm -f "${PROBE}"
if [ "${found}" = "2" ]; then
    pass "the lint finds exactly the two real offenders, and none of: the comment, a logical-OR grep -q, grep -q on a file, or grep -q on a here-string"
else
    fail "the lint found ${found} offenders in a probe that contains exactly 2 — it is not discriminating"
fi

echo
echo "=== 4. gen_password does not abort, on either branch ==="
# The lint is structural; this is the behaviour. gen_password is the site where
# the defect was fatal and deterministic, so it is checked directly.
run_gen() {
    bash -c '
        set -euo pipefail
        # shellcheck disable=SC1090
        source <(sed -n "/^gen_password()/,/^}/p" "'"${REPO_ROOT}"'/installer/lib/70-station.sh")
        fatal() { echo "FATAL: $*" >&2; exit 1; }
        CONFIG_FILE=/etc/birdnet/birdnet.conf
        '"$1"'
        gen_password
    ' 2>/dev/null
}

for label in "with openssl:" "without openssl:"; do
    if [ "${label}" = "without openssl:" ]; then
        # Shadow `command -v openssl` so the /dev/urandom branch is taken.
        setup='command() { if [ "${2:-}" = "openssl" ]; then return 1; fi; builtin command "$@"; }'
    else
        setup=':'
    fi
    aborts=0
    wrong=0
    for _ in 1 2 3 4 5 6 7 8 9 10; do
        p="$(run_gen "${setup}")" || aborts=$((aborts + 1))
        [ "${#p}" -ne 22 ] && wrong=$((wrong + 1))
    done
    if [ "${aborts}" -eq 0 ] && [ "${wrong}" -eq 0 ]; then
        pass "${label} 10/10 runs produced a 22-character password"
    else
        fail "${label} ${aborts}/10 aborted, ${wrong}/10 were not 22 characters"
    fi
done

echo
if [ "${FAILED}" -eq 0 ]; then
    echo "pipefail-sigpipe: all pass"
else
    echo "pipefail-sigpipe: FAILURES"
fi
exit "${FAILED}"
