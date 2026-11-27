#!/usr/bin/env bash
# installer/test/quickstart-release.sh — does the Docker quick start install a
# published release, not the development branch? (OOB-22)
#
# quickstart.sh fetched the compose files from `main` and wrote
# BIRDNET_IMAGE_TAG=latest, so a station started from it ran whatever the
# branch and the newest image held that minute. It now resolves one release
# (BIRDNET_VERSION, or the newest published one) and takes both the compose
# files and the image tag from it:
#
#   Q1  the newest release is read from the GitHub API answer, `v` dropped.
#   Q2  BIRDNET_VERSION wins, with or without a `v`, and asks no API.
#   Q3  an answer with no tag, or a tag that is not a version, is no release.
#   Q4  the compose files come from that tag and the image is pinned to it.
#
# What this cannot show: quickstart.sh itself is still fetched from `main`
# (publishing it as a release asset is a change to release.yml, outside this
# directory), and the GitHub API was not called.
#
# Usage: installer/test/quickstart-release.sh
# Needs bash + coreutils. No network.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
QS="${REPO_ROOT}/quickstart.sh"
FAILED=0
pass() { printf '  PASS  %s\n' "$*"; }
fail() { printf '  FAIL  %s\n' "$*"; FAILED=1; }
WORK="$(mktemp -d)"
trap 'rm -rf "${WORK}"' EXIT

fn="$(sed -n '/^resolve_release()/,/^}/p' "${QS}")"
if [ -z "${fn}" ]; then
    fail "quickstart.sh has no resolve_release: it installs whatever main and :latest hold"
fi

# $1 = what the stubbed API answers ("" = the call fails). Prints the release
# or "NONE"; logs each curl call to ${WORK}/calls.
resolve() {
    (
        # Read by the eval'd resolve_release, which shellcheck cannot see.
        # shellcheck disable=SC2034
        REPO="tomtom215/BirdNet-Behavior"
        API_ANSWER="$1"
        curl() { echo "curl $*" >>"${WORK}/calls"; [ -n "${API_ANSWER}" ] || return 22; printf '%s' "${API_ANSWER}"; }
        eval "${fn}"
        resolve_release || echo NONE
    )
}

: >"${WORK}/calls"
got="$(BIRDNET_VERSION='' resolve $'{\n  "url": "x",\n  "tag_name": "v0.19.0",\n  "name": "0.19.0"\n}')"
[ "${got}" = "0.19.0" ] && pass "Q1: the newest release, v dropped" || fail "Q1: got '${got}'"

: >"${WORK}/calls"
got="$(BIRDNET_VERSION=v0.18.2 resolve '{"tag_name": "v0.19.0"}')"
[ "${got}" = "0.18.2" ] && [ ! -s "${WORK}/calls" ] && pass "Q2: BIRDNET_VERSION wins and asks nothing" \
    || fail "Q2: got '${got}', calls: $(cat "${WORK}/calls")"
got="$(BIRDNET_VERSION=0.18.2 resolve '')"
[ "${got}" = "0.18.2" ] && pass "Q2: without a v too" || fail "Q2: got '${got}'"

got="$(BIRDNET_VERSION='' resolve '')"
[ "${got}" = "NONE" ] && pass "Q3: a failed call is no release" || fail "Q3: got '${got}'"
got="$(BIRDNET_VERSION='' resolve '{"message": "API rate limit exceeded"}')"
[ "${got}" = "NONE" ] && pass "Q3: an answer with no tag is no release" || fail "Q3: got '${got}'"
got="$(BIRDNET_VERSION='main' resolve '')"
[ "${got}" = "NONE" ] && pass "Q3: a branch name is not a version" || fail "Q3: got '${got}'"
got="$(BIRDNET_VERSION='0.19.0; rm -rf /' resolve '')"
[ "${got}" = "NONE" ] && pass "Q3: nor is anything with more in it" || fail "Q3: got '${got}'"

if grep -qE '^[[:space:]]*RAW_BASE="https://raw\.githubusercontent\.com/\$\{REPO\}/v\$\{RELEASE\}"' "${QS}" \
    && grep -qE '^    IMAGE_TAG="\$\{RELEASE\}"' "${QS}"; then
    pass "Q4: the compose files and the image tag come from the release"
else
    fail "Q4: the compose files or the image tag do not follow the release"
fi

if [ "${FAILED}" -ne 0 ]; then
    echo "quickstart-release: FAILED"
    exit 1
fi
echo "quickstart-release: all pass"
