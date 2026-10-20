#!/usr/bin/env bash
# scripts/test-build-versioned-docs.sh — does the versioned-docs build publish
# what its header says it publishes?
#
# Runs scripts/build-versioned-docs.sh in a throwaway repository with synthetic
# release tags and a stub `mdbook` that writes down which source tree it was
# asked to render and with what environment. Nothing is rendered and nothing
# touches the network, so it needs only bash, git and python3.
#
# What it pins, each with the situation that made it necessary:
#
#   * `/dev/` is `main` on every run that publishes. docs.yml also runs on a
#     release-tag push, where the checkout is the tag, and the script built
#     `/dev/` from HEAD: the "development" manual was the release's own, and
#     stayed so until the next push to `main`.
#   * A pull request's build still grades the pull request's own tree (the build
#     job runs on PRs so a broken link is caught before it reaches `main`), which
#     is what DOCS_DEV_REF=HEAD asks for. This is the counterpart of the first:
#     a script that always built origin/main would pass that one and stop gating
#     pull requests.
#   * A release is rendered from its own tag, in version order (`sort -V`, so
#     v0.17.10 is newer than v0.17.2), without pre-releases and without anything
#     before the first versioned release.
#   * The site root is the latest release's manual, and every tree is
#     link-checked at the base URL it will be served from.
#   * "Suggest an edit" exists on `/dev/` only.
#   * An edit that is not committed cannot reach the site.
#
# The script under test defaults to scripts/build-versioned-docs.sh;
# BUILD_DOCS_SCRIPT points at another copy (this is how the test is shown to
# fail against the script as it was before the fix).
#
# Usage: scripts/test-build-versioned-docs.sh
# Exit 0 = every assertion held.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="${BUILD_DOCS_SCRIPT:-${REPO_ROOT}/scripts/build-versioned-docs.sh}"
FAILED=0
PASSED=0
pass() { printf '  PASS  %s\n' "$*"; PASSED=$((PASSED + 1)); }
fail() { printf '  FAIL  %s\n' "$*"; FAILED=$((FAILED + 1)); }

# expect <what> <got> <want>
expect() {
    if [ "$2" = "$3" ]; then
        pass "$1"
    else
        fail "$1: got '$2', want '$3'"
    fi
}

SANDBOX="$(mktemp -d)"
trap 'rm -rf "${SANDBOX}"' EXIT
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null
export LINKLOG="${SANDBOX}/linkcheck.log"

# ── the stub mdbook ─────────────────────────────────────────────────────────
# `mdbook build -d <dest>`, run from the source tree. It copies the tree's
# marker.txt into <dest> (which source did we render?) and records the
# environment the script gave it.
mkdir -p "${SANDBOX}/bin"
cat > "${SANDBOX}/bin/mdbook" <<'STUB'
#!/usr/bin/env bash
[ "${1:-}" = build ] && [ "${2:-}" = -d ] || { echo "stub mdbook: unexpected args: $*" >&2; exit 64; }
dest="$3"
mkdir -p "${dest}"
cp marker.txt "${dest}/built-from.txt"
printf '%s' "${MDBOOK_OUTPUT__HTML__SITE_URL-}" > "${dest}/site-url.txt"
if [ "${MDBOOK_OUTPUT__HTML__EDIT_URL_TEMPLATE+set}" = set ]; then
    printf 'overridden:%s' "${MDBOOK_OUTPUT__HTML__EDIT_URL_TEMPLATE}" > "${dest}/edit-url.txt"
else
    printf 'default' > "${dest}/edit-url.txt"
fi
cp docs/book-theme/version-id.js "${dest}/version-id.js"
printf '<html></html>\n' > "${dest}/index.html"
STUB
chmod +x "${SANDBOX}/bin/mdbook"

# ── the synthetic repository ────────────────────────────────────────────────
# One commit per release, each with a marker.txt naming itself, then one more
# commit on main after the last release (the unreleased manual).
FIXTURE="${SANDBOX}/repo"
mkdir -p "${FIXTURE}"
cd "${FIXTURE}" || exit 1
git init -q -b main .
git config user.email t@example.invalid
git config user.name t
mkdir -p docs/book-theme scripts
for f in version-switcher.js version-switcher.css a11y.js; do
    printf '/* %s */\n' "$f" > "docs/book-theme/$f"
done
# The link checker is a stub too: it records what it was asked to check.
cat > scripts/check-book-links.py <<'PY'
import os
import sys

with open(os.environ["LINKLOG"], "a", encoding="utf-8") as log:
    log.write(" ".join(sys.argv[1:]) + "\n")
PY
git add -A
commit_as() { # <marker> [<tag>...]
    printf '%s\n' "$1" > marker.txt
    git add marker.txt
    git commit -q -m "$1"
    shift
    local t
    for t in "$@"; do git tag -a -m "Release $t" "$t"; done
}
commit_as v0.9.0 v0.9.0                       # before the first versioned release
commit_as v0.17.1 v0.17.1
commit_as v0.17.2 v0.17.2
commit_as v0.17.10 v0.17.10                   # 0.17.10 is newer than 0.17.2
commit_as v0.18.0-rc.1 v0.18.0-rc.1           # a pre-release: no manual of its own
commit_as v0.18.0 v0.18.0
commit_as main-after-v0.18.0                  # unreleased: the tip of main
MAIN_TIP="$(git rev-parse HEAD)"
git update-ref refs/remotes/origin/main "${MAIN_TIP}"   # as a full checkout has it
# A pull request: a commit that is not on main, branched from the last release.
git checkout -q -b pr v0.18.0
commit_as pull-request-edit
PR_TIP="$(git rev-parse HEAD)"
git checkout -q main

# build <name> <checkout> [VAR=value ...] — run the script on that checkout into
# ${SANDBOX}/out-<name>. The checkout is detached the way a tag or pull-request
# run leaves it; sets BUILD_RC and OUT.
build() {
    local name="$1" checkout="$2"
    shift 2
    OUT="${SANDBOX}/out-${name}"
    : > "${LINKLOG}"
    git -C "${FIXTURE}" checkout -q --detach "${checkout}"
    ( cd "${FIXTURE}" && env PATH="${SANDBOX}/bin:${PATH}" "$@" "${SCRIPT}" "${OUT}" ) \
        > "${SANDBOX}/${name}.log" 2>&1
    BUILD_RC=$?
}
read_file() { cat "$1" 2>/dev/null || printf '(missing)'; }
exists() { [ -e "$1" ] && printf yes || printf no; }

# ── 1. a release-tag push ───────────────────────────────────────────────────
# The checkout is the tag, the way actions/checkout leaves a tag run; main has
# moved on since. docs.yml sets no DOCS_DEV_REF here.
echo "=== a release-tag push (HEAD is v0.18.0, main is one commit ahead) ==="
build tag v0.18.0
expect "the build succeeds" "${BUILD_RC}" 0
expect "/dev/ is the tip of main, not the release" \
    "$(read_file "${OUT}/dev/built-from.txt")" "main-after-v0.18.0"
expect "the site root is the latest release's manual" \
    "$(read_file "${OUT}/built-from.txt")" "v0.18.0"
for t in v0.17.1 v0.17.2 v0.17.10 v0.18.0; do
    expect "/${t}/ is rendered from the tag ${t}" \
        "$(read_file "${OUT}/${t}/built-from.txt")" "${t}"
done
expect "no manual before the first versioned release" "$(exists "${OUT}/v0.9.0")" no
expect "no manual for a pre-release" "$(exists "${OUT}/v0.18.0-rc.1")" no
expect "versions.json: latest, newest first (sort -V), dev last" \
    "$(python3 -I -c 'import json,sys; d=json.load(open(sys.argv[1])); print(d["latest"], *[v["name"] for v in d["versions"]])' \
        "${OUT}/versions.json" 2>&1)" \
    "v0.18.0 v0.18.0 v0.17.10 v0.17.2 v0.17.1 dev"
expect "/dev/ links to the dev site path" \
    "$(read_file "${OUT}/dev/site-url.txt")" "/BirdNet-Behavior/dev/"
expect "a release is served under its own path" \
    "$(read_file "${OUT}/v0.17.2/site-url.txt")" "/BirdNet-Behavior/v0.17.2/"
expect "the root is served at the site root" \
    "$(read_file "${OUT}/site-url.txt")" "/BirdNet-Behavior/"
expect "the version switcher on /dev/ is told it is dev, one level down" \
    "$(read_file "${OUT}/dev/version-id.js")" 'window.bnbDocs = { version: "dev", depth: 1 };'
expect "\"Suggest an edit\" is left on for /dev/" \
    "$(read_file "${OUT}/dev/edit-url.txt")" default
expect "\"Suggest an edit\" is off for a release" \
    "$(read_file "${OUT}/v0.17.2/edit-url.txt")" 'overridden:""'
expect "every tree is link-checked: 4 releases, the root and /dev/ (6 runs)" \
    "$(wc -l < "${LINKLOG}" | tr -d ' ')" 6
expect "the link check of /dev/ uses the dev base URL" \
    "$(grep -c -- '--base /BirdNet-Behavior/dev/ ' "${LINKLOG}")" 1

# ── 2. a push to main ───────────────────────────────────────────────────────
echo "=== a push to main ==="
build main "${MAIN_TIP}"
expect "the build succeeds" "${BUILD_RC}" 0
expect "/dev/ is the tip of main" \
    "$(read_file "${OUT}/dev/built-from.txt")" "main-after-v0.18.0"
expect "the site root is still the latest release" \
    "$(read_file "${OUT}/built-from.txt")" "v0.18.0"

# ── 3. a pull request ───────────────────────────────────────────────────────
# The counterpart of 1. The build job runs on pull requests so a broken link is
# caught before main has it; that only works if /dev/ is the pull request's tree.
echo "=== a pull request (docs.yml sets DOCS_DEV_REF=HEAD) ==="
build pr "${PR_TIP}" DOCS_DEV_REF=HEAD
expect "the build succeeds" "${BUILD_RC}" 0
expect "/dev/ is the pull request's own tree" \
    "$(read_file "${OUT}/dev/built-from.txt")" "pull-request-edit"
expect "the releases are still built from their tags" \
    "$(read_file "${OUT}/v0.18.0/built-from.txt")" "v0.18.0"

# ── 4. nothing to call main ─────────────────────────────────────────────────
# A checkout without origin/main (a shallow clone, a local run in a fresh
# repository) must say so rather than publish the wrong branch as /dev/.
echo "=== no origin/main to build /dev/ from ==="
git -C "${FIXTURE}" update-ref -d refs/remotes/origin/main
build nomain v0.18.0
expect "the build is refused" "$([ "${BUILD_RC}" -ne 0 ] && echo refused || echo built)" refused
if grep -q 'origin/main' "${SANDBOX}/nomain.log"; then
    pass "and it names origin/main, so the reader knows what to fetch"
else
    fail "and it names origin/main, so the reader knows what to fetch (output: $(cat "${SANDBOX}/nomain.log"))"
fi
git -C "${FIXTURE}" update-ref refs/remotes/origin/main "${MAIN_TIP}"

# ── 5. uncommitted edits ────────────────────────────────────────────────────
echo "=== an uncommitted edit on the checkout ==="
git -C "${FIXTURE}" checkout -q --detach "${MAIN_TIP}"
printf 'uncommitted\n' > "${FIXTURE}/marker.txt"
printf 'stray\n' > "${FIXTURE}/stray.txt"
OUT="${SANDBOX}/out-dirty"
: > "${LINKLOG}"
( cd "${FIXTURE}" && env PATH="${SANDBOX}/bin:${PATH}" DOCS_DEV_REF=HEAD "${SCRIPT}" "${OUT}" ) \
    > "${SANDBOX}/dirty.log" 2>&1
expect "the build succeeds" "$?" 0
expect "/dev/ is the committed state, not the working tree" \
    "$(read_file "${OUT}/dev/built-from.txt")" "main-after-v0.18.0"
git -C "${FIXTURE}" checkout -q -- marker.txt
rm -f "${FIXTURE}/stray.txt"

echo
echo "${PASSED} passed, ${FAILED} failed"
[ "${FAILED}" -eq 0 ] || exit 1
echo "OK: the versioned-docs build behaves as documented"
