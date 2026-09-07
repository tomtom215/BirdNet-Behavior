#!/usr/bin/env bash
# installer/test/blank-env-opt-outs.sh — the container's blank-env scrubber must
# keep the two blanks that mean something and drop the ones that mean nothing.
#
# docker/strip-blank-env.sh unsets every blank `BIRDNET_*` variable before the
# entrypoint runs the binary, because clap reads an empty environment variable
# as a supplied value and `BIRDNET_LATITUDE=` then fails to parse (F-1). Two
# blanks are the documented opt-outs and must survive:
#
#   BIRDNET_IMAGE_CACHE_DIR=   no species-image cache (air-gapped stations)
#   BIRDNET_ANALYTICS_DB=      no DuckDB analytics (very low-RAM boards)
#
# The second was missing from the exception list, so a compose override that
# blanked it was silently turned back into the default `<db>.duckdb` — the
# only runtime opt-out for analytics was unreachable from Docker, as it was
# unreachable from the command line until `src/cli.rs` gave `--analytics-db`
# the OsString parser `--image-cache-dir` already had.
#
# Observed failing against the shipped scrubber:
#   FAIL  BIRDNET_ANALYTICS_DB= was scrubbed; the analytics opt-out is unreachable from Docker
#
# Usage: installer/test/blank-env-opt-outs.sh
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LIB="${HERE}/../../docker/strip-blank-env.sh"
FAILED=0
pass() { echo "  PASS  $*"; }
fail() { echo "  FAIL  $*"; FAILED=1; }

# Run the scrubber in a subshell with a controlled environment and report,
# for each name, whether it is still set afterwards ("set" / "unset").
probe() {
    env -i PATH="$PATH" \
        BIRDNET_IMAGE_CACHE_DIR= \
        BIRDNET_ANALYTICS_DB= \
        BIRDNET_LATITUDE= \
        BIRDNET_MODEL_DIR=/data/model \
        bash -c '
            # shellcheck disable=SC1090
            . "$1" 2>/dev/null
            strip_blank_birdnet_env 2>/dev/null
            for n in BIRDNET_IMAGE_CACHE_DIR BIRDNET_ANALYTICS_DB BIRDNET_LATITUDE BIRDNET_MODEL_DIR; do
                if [ "${!n+set}" = set ]; then echo "$n set"; else echo "$n unset"; fi
            done
        ' _ "${LIB}"
}
OUT="$(probe)"

echo "=== the two documented opt-outs survive as set-but-empty ==="
if grep -q '^BIRDNET_IMAGE_CACHE_DIR set$' <<<"${OUT}"; then
    pass "BIRDNET_IMAGE_CACHE_DIR= survives"
else
    fail "BIRDNET_IMAGE_CACHE_DIR= was scrubbed; the image-cache opt-out is unreachable from Docker"
fi
if grep -q '^BIRDNET_ANALYTICS_DB set$' <<<"${OUT}"; then
    pass "BIRDNET_ANALYTICS_DB= survives"
else
    fail "BIRDNET_ANALYTICS_DB= was scrubbed; the analytics opt-out is unreachable from Docker"
fi

echo "=== counterpart: a meaningless blank is dropped and a real value is kept ==="
if grep -q '^BIRDNET_LATITUDE unset$' <<<"${OUT}"; then
    pass "BIRDNET_LATITUDE= is dropped (clap would reject the empty value)"
else
    fail "BIRDNET_LATITUDE= survived; the daemon would exit 2 at argument parsing"
fi
if grep -q '^BIRDNET_MODEL_DIR set$' <<<"${OUT}"; then
    pass "a non-blank value is left alone"
else
    fail "a non-blank value was scrubbed"
fi

if [ "${FAILED}" -eq 0 ]; then echo; echo "blank-env-opt-outs: all pass"; else echo; echo "blank-env-opt-outs: FAILURES"; fi
exit "${FAILED}"
