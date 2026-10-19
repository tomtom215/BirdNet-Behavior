#!/usr/bin/env bash
# scripts/check-release-tag.sh — is this tag one that may publish an image?
#
# Usage: scripts/check-release-tag.sh <tag> [<default-branch-ref>]
#        e.g. scripts/check-release-tag.sh v0.18.0 origin/main
#
# The image workflow publishes and signs on any tag push that matches its filter
# (`v[0-9]+.[0-9]+.[0-9]*`, which also matches `v1.2.3foo`), and it runs in
# parallel with release.yml rather than after it, so a tag that release.yml's
# `validate` job rejects still pushed `X.Y.Z`, `X.Y`, `X` and `latest` and signed
# them. This applies the checks of `validate` that need no token and no network
# beyond the checkout:
#
#   1. the tag is `vX.Y.Z` or `vX.Y.Z-pre.N` exactly;
#   2. X.Y.Z is the workspace version in Cargo.toml (a tag on a commit whose
#      manifest says something else would publish the wrong build under the tag);
#   3. the tagged commit is on the default branch (a tag pushed from a side branch
#      is not a release).
#
# What it does not check: that the tag is annotated, or that CHANGELOG, CITATION
# and openapi.json agree. release.yml's `validate` does, through `gh api`, and a
# release it rejects has no GitHub Release; the image for such a tag is at least
# one built from the default branch at the version its manifest names.
#
# Exit 0 = may publish. Anything else prints why on stderr.
set -euo pipefail

tag="${1:?usage: check-release-tag.sh <tag> [<default-branch-ref>]}"
default_ref="${2:-origin/main}"

fail() { echo "::error::$*" >&2; exit 1; }

version="${tag#v}"
re='^[0-9]+\.[0-9]+\.[0-9]+(-[A-Za-z0-9.]+)?$'
[[ "${tag}" == v* && "${version}" =~ ${re} ]] \
    || fail "'${tag}' is not vX.Y.Z or vX.Y.Z-pre.N, so no image is published for it."

manifest="$(awk '/^\[workspace\.package\]/{flag=1; next}
                 /^\[/{flag=0}
                 flag && /^version = /{gsub(/[" ]/, "", $3); print $3; exit}' Cargo.toml)"
[[ "${manifest}" == "${version}" ]] \
    || fail "'${tag}' is version ${version} but Cargo.toml says '${manifest}'. Tag the commit that bumps the version."

if ! git rev-parse --verify --quiet "${default_ref}^{commit}" >/dev/null; then
    fail "cannot find ${default_ref} to check that ${tag} is on it; fetch it first."
fi
commit="$(git rev-parse "refs/tags/${tag}^{commit}" 2>/dev/null || git rev-parse HEAD)"
git merge-base --is-ancestor "${commit}" "${default_ref}" \
    || fail "${tag} (${commit:0:12}) is not on ${default_ref}, so no image is published for it."

echo "${tag}: version matches Cargo.toml and the commit is on ${default_ref}."
