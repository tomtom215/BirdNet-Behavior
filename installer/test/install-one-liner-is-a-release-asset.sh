#!/usr/bin/env bash
# installer/test/install-one-liner-is-a-release-asset.sh — does anything a reader
# is told to paste into a root shell fetch the installer from a branch?
#
# `curl …/raw.githubusercontent.com/<repo>/main/install.sh | sudo bash` runs
# whatever `main` holds at that second, as root, on the reader's machine: a push
# to the branch is a change to what every new install executes, with no release,
# no checksum and no provenance in between. The release workflow attaches
# `install.sh` to every release, checksums it in SHA256SUMS and attests it, and
# `https://github.com/<repo>/releases/latest/download/install.sh` is the stable
# address of the newest one — so that is what the README, the manual, the
# installer's own usage text and its error messages should say.
#
# Counterpart: the release-asset form is accepted, and a scan that matched
# nothing at all (a layout change that moved the files) fails rather than
# passes.
#
# Not covered, on purpose: `quickstart.sh` (the Docker path) is fetched from
# `main` too, and so are the compose files it downloads; neither is a release
# asset yet, so switching its one-liner before a release carries them would
# break it. `scripts/hardware-test.sh` and its page fetch `main` deliberately:
# a maintainer's harness that exercises the branch's tip. Both are listed in
# docs/HANDOFF.md.
#
# Usage: installer/test/install-one-liner-is-a-release-asset.sh
# Needs only bash and grep. Exit 0 = all pass.

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
FAILED=0
pass() { printf '  PASS  %s\n' "$*"; }
fail() { printf '  FAIL  %s\n' "$*"; FAILED=1; }

# Where a reader is sent: README, the manual's sources, the architecture notes,
# the installer's sources and the script they build.
SCAN=(README.md docs/book docs/architecture installer/lib install.sh packaging)
BRANCH_FORM='raw\.githubusercontent\.com/[^[:space:]]*/(main|master)/install\.sh'
ASSET_FORM='github\.com/[^[:space:]]*/releases/(latest/download|download/[^/[:space:]]+)/install\.sh'

scan() { # pattern → "file:line:text" for each match under SCAN, minus rendered copies
    (cd "${ROOT}" && grep -rnE "$1" "${SCAN[@]}" 2>/dev/null | grep -v '/_generated/')
}

echo "== no branch-pinned installer one-liner =="
hits="$(scan "${BRANCH_FORM}")"
if [ -z "${hits}" ]; then
    pass "nothing under ${SCAN[*]} fetches install.sh from main or master"
else
    fail "these fetch install.sh from a branch:"
    printf '%s\n' "${hits}" | cut -c1-200 | sed 's/^/        /'
fi

echo "== the release-asset form is what they say instead =="
n="$(scan "${ASSET_FORM}" | wc -l | tr -d ' ')"
if [ "${n}" -ge 8 ]; then
    pass "${n} places name the release asset"
else
    fail "only ${n} places name a release asset (expected at least 8): the scan may have stopped seeing the files"
fi

echo "== the scan itself =="
if printf '%s\n' 'curl -fsSL https://raw.githubusercontent.com/o/r/main/install.sh | sudo bash' | grep -qE "${BRANCH_FORM}"; then
    pass "the branch pattern matches a branch one-liner"
else
    fail "the branch pattern does not match a branch one-liner"
fi
if printf '%s\n' 'curl -fsSL https://github.com/o/r/releases/latest/download/install.sh | sudo bash' | grep -qE "${BRANCH_FORM}"; then
    fail "the branch pattern matches the release-asset form"
else
    pass "the branch pattern leaves the release-asset form alone"
fi

[ "${FAILED}" -eq 0 ] && echo "ALL PASS" || { echo "FAILED"; exit 1; }
