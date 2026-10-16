#!/usr/bin/env bash
# installer/test/release-tag-gate.sh — does the image workflow's tag gate refuse
# what release.yml's validation would, and accept what it would?
#
# Runs scripts/check-release-tag.sh in a throwaway repository with a manifest, a
# default branch and tags in each state the gate distinguishes. The "refuses"
# cases each have a counterpart that accepts, so a gate that refuses everything
# cannot pass.
#
# Usage: installer/test/release-tag-gate.sh
# Needs only bash, git and awk. Exit 0 = all pass.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
GATE="${REPO_ROOT}/scripts/check-release-tag.sh"
FAILED=0
pass() { printf '  PASS  %s\n' "$*"; }
fail() { printf '  FAIL  %s\n' "$*"; FAILED=1; }

SANDBOX="$(mktemp -d)"
trap 'rm -rf "${SANDBOX}"' EXIT
cd "${SANDBOX}" || exit 1
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null
git init -q -b main .
git config user.email t@example.invalid
git config user.name t
printf '[workspace.package]\nversion = "1.4.0"\n' > Cargo.toml
git add Cargo.toml && git commit -q -m "bump to 1.4.0"
git tag v1.4.0
git tag v1.4.0foo
git tag v1.5.0          # same commit, but the manifest there says 1.4.0
printf '[workspace.package]\nversion = "1.5.0-rc.1"\n' > Cargo.toml
git commit -q -am "bump to 1.5.0-rc.1"
git tag v1.5.0-rc.1
# `origin/main` as a checkout would have it.
git update-ref refs/remotes/origin/main HEAD

# A commit that is not on the default branch.
git checkout -q -b side v1.4.0
printf '[workspace.package]\nversion = "1.6.0"\n' > Cargo.toml
git commit -q -am "bump to 1.6.0 on a side branch"
git tag v1.6.0

# The workflow runs in a checkout of the tag, so each case checks the tag out.
expect_ok() {
    local tag="$1" why="$2" out
    git checkout -q --detach "refs/tags/${tag}" 2>/dev/null || git checkout -q --detach v1.4.0
    if out="$(bash "${GATE}" "${tag}" origin/main 2>&1)"; then pass "${tag}: ${why}"
    else fail "${tag} should publish (${why}) but was refused: ${out}"; fi
}
expect_refused() {
    local tag="$1" why="$2" out
    git checkout -q --detach "refs/tags/${tag}" 2>/dev/null || git checkout -q --detach v1.4.0
    if out="$(bash "${GATE}" "${tag}" origin/main 2>&1)"; then fail "${tag} should be refused (${why}) but publishes"
    else pass "${tag}: refused, ${why}"; fi
}

expect_ok v1.4.0 "a release on the default branch whose manifest agrees"
expect_ok v1.5.0-rc.1 "a pre-release whose manifest names that pre-release version"
expect_refused v1.4.0foo "the workflow's tag glob matches it; it is not a version"
expect_refused v1.5.0 "the manifest says 1.4.0"
expect_refused v1.6.0 "its commit is not on origin/main"
expect_refused 1.4.0 "no leading v"
exit "${FAILED}"
