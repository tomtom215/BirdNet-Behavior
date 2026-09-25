#!/usr/bin/env bash
# installer/test/geomodel-fallback-quiet.sh — is the documented geomodel
# fallback reported as what it is, and is a real failure still loud?
#
# RELEASING.md documents that the models release may not carry the geomodel
# yet, in which case every install fetches it from upstream birdnet-team. That
# is the expected path, and both front doors reported it as a fault:
#
#   * the installer's plain download ran `curl -fsSL`, so the operator saw
#     "curl: (22) The requested URL returned error: 404" and then a [WARN]
#     "download from GitHub release … failed";
#   * the container's fetch printed its whole first-run banner for the missing
#     file ("Typical download: 1–3 min on fibre…", for a 14 MB file that was
#     not there), then curl's 404 and two WARNINGs.
#
# The counterpart matters as much: an origin that answers 500 is a real
# failure and must still be reported as one, so "quiet" cannot mean "mute".
#
# Both halves drive the shipping functions with `curl` replaced on PATH by a
# stub that behaves like curl for the flags these scripts use: the mirror URL
# answers ${MIRROR_STATUS}, the upstream URL answers 200 with the real bytes.
#
# Usage: installer/test/geomodel-fallback-quiet.sh
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
LIB="${REPO_ROOT}/installer/lib"
ENTRYPOINT="${REPO_ROOT}/docker/entrypoint.sh"
FAILED=0
pass() { printf '  PASS  %s\n' "$*"; }
fail() { printf '  FAIL  %s\n' "$*"; FAILED=1; }

WORK="$(mktemp -d)"
trap 'rm -rf "${WORK}"' EXIT

BODY='the geomodel bytes'
BODY_SHA="$(printf '%s' "${BODY}" | sha256sum | awk '{print $1}')"

mkdir -p "${WORK}/bin"
cat >"${WORK}/bin/curl" <<'STUB'
#!/usr/bin/env bash
# A curl for these tests: parses the flags install.sh / entrypoint.sh pass.
fail_on_http=0 show_error=0 head=0 out="" wfmt="" url=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        -o|--output) out="$2"; shift ;;
        -w|--write-out) wfmt="$2"; shift ;;
        -C|--continue-at|--retry|--retry-delay|--retry-max-time|--connect-timeout|--max-time) shift ;;
        --fail) fail_on_http=1 ;;
        --show-error) show_error=1 ;;
        --head) head=1 ;;
        --*) ;;
        -*) f="${1#-}"
            case "$f" in *f*) fail_on_http=1 ;; esac
            case "$f" in *S*) show_error=1 ;; esac
            case "$f" in *I*) head=1 ;; esac ;;
        *) url="$1" ;;
    esac
    shift
done
echo "${url}" >>"${STUB_LOG}"
case "${url}" in
    */mirror/*) code="${MIRROR_STATUS}" ;;
    *) code=200 ;;
esac
if [ "${head}" = 1 ]; then
    {
        printf 'HTTP/2 %s\r\n' "${code}"
        [ "${code}" = 200 ] && printf 'content-length: 14000000\r\n'
        printf '\r\n'
    } >"${out:-/dev/stdout}"
    [ -n "${wfmt}" ] && printf '%s' "${code}"
    exit 0
fi
if [ "${code}" != 200 ] && [ "${fail_on_http}" = 1 ]; then
    [ "${show_error}" = 1 ] && echo "curl: (22) The requested URL returned error: ${code}" >&2
    exit 22
fi
if [ "${code}" = 200 ]; then body="${STUB_BODY}"; else body="<html>${code}</html>"; fi
if [ -n "${out}" ]; then printf '%s' "${body}" >"${out}"; else printf '%s' "${body}"; fi
[ -n "${wfmt}" ] && printf '%s' "${code}"
exit 0
STUB
chmod +x "${WORK}/bin/curl"
export STUB_BODY="${BODY}" STUB_LOG="${WORK}/curl.log"

# ── the installer ───────────────────────────────────────────────────────────
run_installer() { # $1=mirror status -> output in ${WORK}/inst.out, file in ${WORK}/inst/geo
    rm -rf "${WORK}/inst"; mkdir -p "${WORK}/inst"; : >"${STUB_LOG}"
    (
        set -euo pipefail
        export MIRROR_STATUS="$1" PATH="${WORK}/bin:${PATH}"
        # shellcheck disable=SC1090
        source <(sed -n '/^download()/,/^}/p;/^download_or_absent()/,/^}/p' "${LIB}/40-download.sh")
        # shellcheck disable=SC1090
        source <(sed -n '/^verify_model_sha256()/,/^}/p;/^fetch_verified_model()/,/^}/p' "${LIB}/55-model.sh")
        info()    { echo "[INFO] $*"; }
        success() { echo "[OK] $*"; }
        warn()    { echo "[WARN] $*"; }
        fatal()   { echo "[FATAL] $*"; exit 1; }
        fetch_verified_model "${WORK}/inst/geo" "${BODY_SHA}" "geomodel (~14 MB)" 0 \
            "GitHub release models-test" "https://example.invalid/mirror/geo.onnx" \
            "upstream birdnet-team/geomodel vtest" "https://example.invalid/upstream/geo.onnx"
    ) >"${WORK}/inst.out" 2>&1
}

echo "=== installer: the models release without the geomodel is not an error ==="
if run_installer 404; then pass "the geomodel was fetched"; else fail "the fetch failed: $(cat "${WORK}/inst.out")"; fi
if grep -q "upstream/geo.onnx" "${STUB_LOG}" && grep -q "mirror/geo.onnx" "${STUB_LOG}"; then
    pass "precondition: the mirror was asked first, then upstream"
else
    fail "precondition: the stub did not see both origins: $(cat "${STUB_LOG}")"
fi
if grep -qE 'curl: \(22\)|error: 404' "${WORK}/inst.out"; then
    fail "curl's 404 error reached the operator: $(grep -E 'curl|404' "${WORK}/inst.out")"
else
    pass "no curl error for the expected 404"
fi
if grep -q '^\[WARN\]' "${WORK}/inst.out"; then
    fail "the expected fallback was reported as a warning: $(grep '^\[WARN\]' "${WORK}/inst.out")"
else
    pass "no [WARN] for the expected fallback"
fi
if grep -qi 'not published at GitHub release' "${WORK}/inst.out"; then
    pass "one informational line says where it was not found"
else
    fail "the fallback is not explained: $(cat "${WORK}/inst.out")"
fi

echo "=== installer counterpart: a mirror answering 500 is still a warning ==="
run_installer 500 || true
if grep -q '^\[WARN\].*GitHub release' "${WORK}/inst.out"; then
    pass "a real failure still warns"
else
    fail "a 500 from the mirror went unreported: $(cat "${WORK}/inst.out")"
fi

# ── the container ───────────────────────────────────────────────────────────
run_container() { # $1=mirror status -> output in ${WORK}/ctr.out
    rm -rf "${WORK}/ctr"; mkdir -p "${WORK}/ctr"; : >"${STUB_LOG}"
    (
        set -eu
        export MIRROR_STATUS="$1" PATH="${WORK}/bin:${PATH}"
        # shellcheck disable=SC1090
        source <(sed -n '/^rule()/p;/^human_bytes()/,/^}/p;/^remote_size()/,/^}/p;/^remote_status()/,/^}/p;/^verify_sha256()/,/^}/p;/^fetch_one()/,/^}/p;/^ensure_geomodel_file()/,/^}/p' "${ENTRYPOINT}")
        log()  { echo "[birdnet] $*"; }
        warn() { echo "[birdnet] WARNING: $*"; }
        # shellcheck disable=SC2034  # read by the sourced functions
        GH_BASE="https://example.invalid/mirror"
        # shellcheck disable=SC2034
        GEOMODEL_UPSTREAM_BASE="https://example.invalid/upstream"
        # shellcheck disable=SC2034
        MODEL_RELEASE_TAG="models-test"
        # shellcheck disable=SC2034
        GEOMODEL_VERSION="vtest"
        ensure_geomodel_file "${WORK}/ctr/geo.onnx" "geo.onnx" "${BODY_SHA}" "geomodel (~14 MB)"
    ) >"${WORK}/ctr.out" 2>&1
}

echo "=== container: the models release without the geomodel is not an error ==="
if run_container 404; then pass "the geomodel was fetched"; else fail "the fetch failed: $(cat "${WORK}/ctr.out")"; fi
if grep -q "upstream/geo.onnx" "${STUB_LOG}"; then
    pass "precondition: upstream was asked"
else
    fail "precondition: the stub never saw upstream: $(cat "${STUB_LOG}")"
fi
if grep -qE 'curl: \(22\)|error: 404' "${WORK}/ctr.out"; then
    fail "curl's 404 error reached the log: $(grep -E 'curl|404' "${WORK}/ctr.out")"
else
    pass "no curl error for the expected 404"
fi
if grep -q 'WARNING' "${WORK}/ctr.out"; then
    fail "the expected fallback was reported as a warning: $(grep WARNING "${WORK}/ctr.out")"
else
    pass "no WARNING for the expected fallback"
fi
if grep -q 'Typical download: 1' "${WORK}/ctr.out"; then
    fail "a 14 MB file got the 541 MB model's 'Typical download: 1–3 min' banner"
else
    pass "no multi-minute download banner for a 14 MB file"
fi

echo "=== container counterpart: a mirror answering 500 is still a warning ==="
run_container 500 || true
if grep -q 'WARNING.*GitHub release' "${WORK}/ctr.out"; then
    pass "a real failure still warns"
else
    fail "a 500 from the mirror went unreported: $(cat "${WORK}/ctr.out")"
fi

if [ "${FAILED}" -ne 0 ]; then
    echo "geomodel-fallback-quiet: FAILED"
    exit 1
fi
echo "geomodel-fallback-quiet: all pass"
