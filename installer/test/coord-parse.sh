#!/usr/bin/env bash
# installer/test/coord-parse.sh — does the installer keep the coordinates an
# operator actually typed?
#
# This exists because of a real report from a station in Germany: the operator
# entered coordinates at the install prompt, and the written config contained
# neither LATITUDE nor LONGITUDE — just the commented-out examples. No error was
# shown that they noticed. The station then ran with occurrence filtering off,
# reporting species from the wrong continent, and the web onboarding wizard
# redirected them on first load because it (correctly) saw no location.
#
# Two defects converged, and both were in the prompt rather than the writer:
#
#   1. The prompt printed a tip telling the operator to right-click on
#      OpenStreetMap — which hands you a *pair*, "49.4521, 8.6724" — and then
#      offered a single-value field whose validator rejected exactly that.
#   2. On rejection it warned once and fell through. A single [WARN] scrolling
#      away behind a 541 MB model download is indistinguishable from success,
#      so the operator had no reason to think the answer had been discarded.
#
# A decimal comma ("49,4521") was rejected too, though the web settings form
# accepts it — so the same operator typing the same number was told yes in one
# place and silently no in the other.
#
# Usage: installer/test/coord-parse.sh
# Needs only bash + awk. Exit 0 = all pass.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
FAILED=0

pass() { printf '  PASS  %s\n' "$*"; }
fail() { printf '  FAIL  %s\n' "$*"; FAILED=1; }

# Load the real parser out of the real source module, so this test cannot pass
# against a copy that has drifted from what ships.
LIB="${REPO_ROOT}/installer/lib/70-station.sh"
# Both extractions need the directive: `disable` applies to the next line only,
# and the process substitutions are non-constant sources by design — the point
# is to exercise the functions as they ship, not a copy of them.
# shellcheck disable=SC1090
source <(sed -n '/^parse_coords()/,/^}/p' "${LIB}")
# shellcheck disable=SC1090
source <(sed -n '/^valid_coord()/,/^}/p' "${LIB}")

# check <input> <expected> <description>
check() {
    local got
    got="$(parse_coords "$1")"
    if [ "${got}" = "$2" ]; then
        pass "$3"
    else
        fail "$3 — got '${got}', wanted '$2'"
    fi
}

echo "=== 1. the shapes the prompt's own OpenStreetMap tip produces ==="
check "49.4521, 8.6724"    "49.4521 8.6724"  "the pair OSM gives you, comma+space"
check "49.4521,8.6724"     "49.4521 8.6724"  "the same pair with no space"
check "-33.8688, 151.2093" "-33.8688 151.2093" "southern/eastern hemisphere pair"

echo
echo "=== 2. decimal comma, as the web settings form already accepts ==="
check "49,4521"            "49.4521"         "a lone decimal-comma latitude"
check "49,4521 8,6724"     "49.4521 8.6724"  "decimal commas, space separated"
check "49,4521,8,6724"     "49.4521 8.6724"  "decimal commas, comma separated"
check "49,4521, 8,6724"    "49.4521 8.6724"  "decimal commas, comma+space"

echo
echo "=== 3. the plain single value, unchanged ==="
check "49.4521"            "49.4521"         "a lone dotted latitude"
check "  49.4521  "        "49.4521"         "surrounding whitespace is trimmed"

echo
echo "=== 4. nonsense is still rejected, not coerced ==="
check "banana"             ""                "a word is not coordinates"
check "999"                ""                "out-of-range latitude rejected"
check "49.4521, 500"       ""                "out-of-range longitude rejected"
check ""                   ""                "empty input stays empty (a real skip)"

echo
echo "=== 5. the ambiguous single comma resolves on range ==="
# "49,4521" cannot be the pair (49, 4521) because 4521 is not a longitude, so
# the comma has to be a decimal point. This is what makes case 2 safe.
check "49,4521"            "49.4521"         "tail out of longitude range → decimal point"
# Where both readings are valid the pair wins, matching the OSM flow; the
# caller echoes it back so the operator can see and correct it.
check "49,45"              "49 45"           "both readings valid → pair, and it is echoed back"

echo
echo "=== 6. counter-test: the previous behaviour on the reported input ==="
# The old prompt ran the raw string straight through valid_coord. This is that
# code path, unchanged, against what the operator pasted.
if valid_coord "49.4521, 8.6724" -90 90; then
    fail "the old validator accepted the OSM pair — then the defect was elsewhere"
else
    pass "the old validator rejected the OSM pair its own tip told you to fetch"
fi
if valid_coord "49,4521" -90 90; then
    fail "the old validator accepted a decimal comma"
else
    pass "…and rejected a decimal comma the web settings form accepts"
fi
# And the new parser accepts both, so the change actually fixes it.
[ -n "$(parse_coords "49.4521, 8.6724")" ] \
    && pass "…while the new parser accepts both — the change is not cosmetic" \
    || fail "the new parser rejects it too — the change fixes nothing"

echo
if [ "${FAILED}" -eq 0 ]; then
    echo "coord-parse: ALL-PASS"
else
    echo "coord-parse: FAILURES"
fi
exit "${FAILED}"
