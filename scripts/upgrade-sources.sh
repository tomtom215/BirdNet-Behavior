#!/usr/bin/env bash
# scripts/upgrade-sources.sh — the published releases a candidate must upgrade
# from, as a JSON array for a workflow matrix.
#
# The newest N published (not draft, not pre-release) releases older than the
# candidate. Older than, not merely different from: a pull request that has not
# bumped the version yet carries the latest release's own number, and upgrading
# a release to itself proves nothing about an upgrade.
#
# Usage: scripts/upgrade-sources.sh CANDIDATE_VERSION [N]
# Needs gh (authenticated via GH_TOKEN) and GITHUB_REPOSITORY.
#   scripts/upgrade-sources.sh 0.17.1      → ["0.17.0","0.16.1","0.16.0"]

set -euo pipefail

cand="${1:?candidate version, e.g. 0.17.1}"
n="${2:-3}"
repo="${GITHUB_REPOSITORY:?set GITHUB_REPOSITORY}"

tags="$(gh api --paginate "repos/${repo}/releases?per_page=100" \
    --jq '.[] | select(.draft | not) | select(.prerelease | not) | .tag_name')"

picked=()
while read -r v; do
    [ -n "${v}" ] && [ "${v}" != "${cand}" ] || continue
    # v < cand: the pair is already in version order, and they differ.
    printf '%s\n%s\n' "${v}" "${cand}" | sort -V -C || continue
    picked+=("${v}")
    [ "${#picked[@]}" -lt "${n}" ] || break
done < <(sed -n 's/^v//p' <<<"${tags}" | sort -V -r)

if [ "${#picked[@]}" -eq 0 ]; then
    echo "upgrade-sources: no published release older than ${cand}" >&2
    exit 1
fi
printf '%s\n' "${picked[@]}" | jq -R . | jq -s -c .
