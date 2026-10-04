#!/usr/bin/env bash
# installer/test/upgrade-e2e.sh — does an in-place upgrade from a published
# release keep the station, and does the rollback the installer prints work?
#
# 0.17.0 shipped with neither question asked. Every installer test sources one
# function against stubs, and the install smoke test runs a fresh install in a
# container with no systemd. Nothing installed an older release, gave it data,
# upgraded it the way an operator does, and started the result under systemd.
# The update printed "roll back with: sudo mv …prev … && sudo systemctl
# restart" over a unit whose preflight the previous binary rejects, so the
# rollback it offered left a station that never started.
#
# This runs the whole operator path on a real systemd host:
#
#   1. the PREVIOUS release's own install.sh installs the PREVIOUS binary
#   2. that binary records detections from a real recording (the magpie
#      fixture), so the database holds rows written by the old code itself
#   3. the CANDIDATE install.sh runs `update` with the CANDIDATE tarball while
#      the service is running
#   4. the service is active again, on the candidate, with every row, a newer
#      schema, and it records a new detection
#   5. the rollback command the candidate installer printed is run verbatim
#   6. the service is active again, on the previous binary, with every row, and
#      it records a new detection against the schema the candidate left behind
#
# It changes the machine it runs on: it installs a service, creates a user and
# writes under /usr/local, /etc and /home. Run it on a disposable host (CI).
#
# Usage (as root, on a host where systemd is PID 1):
#   PREV_VERSION=0.15.0 \
#   CANDIDATE_TARBALL=dist/birdnet-behavior-0.17.1-x86_64-unknown-linux-gnu.tar.gz \
#   installer/test/upgrade-e2e.sh
#
# Optional:
#   CANDIDATE_INSTALLER  install.sh to upgrade with (default: this repo's)
#   PREV_DIR             a directory already holding the previous release's
#                        install.sh and tarball (default: download and verify
#                        them against the release's SHA256SUMS)
#   MODEL_CACHE          directory of model files to stage before installing,
#                        so neither installer downloads ~600 MB; filled from
#                        the install afterwards when it was empty
#   SERVICE_USER         account the station runs as (default: bnbupgrade)

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
REPO="tomtom215/BirdNet-Behavior"
SERVICE="birdnet-behavior"
BIN="/usr/local/bin/birdnet-behavior"
LISTEN="127.0.0.1:8502"
FIXTURE="${REPO_ROOT}/tests/testdata/Pica_pica_30s.wav"

: "${PREV_VERSION:?set PREV_VERSION, e.g. 0.15.0}"
: "${CANDIDATE_TARBALL:?set CANDIDATE_TARBALL to the candidate release tarball}"
CANDIDATE_INSTALLER="${CANDIDATE_INSTALLER:-${REPO_ROOT}/install.sh}"
SERVICE_USER="${SERVICE_USER:-bnbupgrade}"
MODEL_CACHE="${MODEL_CACHE:-}"

WORK="$(mktemp -d /var/tmp/upgrade-e2e.XXXXXX)"
STEP="setup"

log()  { printf '\n=== %s\n' "$*"; }
pass() { printf '  PASS  %s\n' "$*"; }
# To stderr: record_one runs inside $(…), where stdout is the value.
die() {
    {
        printf '  FAIL  [%s] %s\n' "${STEP}" "$*"
        echo "--- systemctl status"
        systemctl status "${SERVICE}" --no-pager -l 2>&1 | tail -30
        echo "--- journal (last 120 lines)"
        journalctl -u "${SERVICE}" --no-pager -n 120 2>&1
    } >&2
    exit 1
}

[ "$(id -u)" -eq 0 ] || { echo "upgrade-e2e: must run as root"; exit 2; }
[ -d /run/systemd/system ] || { echo "upgrade-e2e: systemd is not running here; this test needs it"; exit 2; }
[ -f "${CANDIDATE_TARBALL}" ] || { echo "upgrade-e2e: no tarball at ${CANDIDATE_TARBALL}"; exit 2; }
[ -f "${FIXTURE}" ] || { echo "upgrade-e2e: missing fixture ${FIXTURE}"; exit 2; }
if [ -e "${BIN}" ] || [ -e "/etc/systemd/system/${SERVICE}.service" ]; then
    echo "upgrade-e2e: a station is already installed here; this test needs a clean host"
    exit 2
fi

CAND_VERSION="$(basename "${CANDIDATE_TARBALL}" | sed -E 's/^birdnet-behavior-([0-9][^-]*)-.*/\1/')"
ARCH="$(basename "${CANDIDATE_TARBALL}" | sed -E 's/^birdnet-behavior-[0-9][^-]*-(.*)\.tar\.gz$/\1/')"
[ "${CAND_VERSION}" != "${PREV_VERSION}" ] || { echo "upgrade-e2e: candidate and previous are both ${PREV_VERSION}"; exit 2; }

# ── helpers ──────────────────────────────────────────────────────────────────

# Total detections as the running station reports them.
detections() {
    curl -fsS "http://${LISTEN}/api/v2/stats" 2>/dev/null \
        | python3 -c 'import json,sys; print(json.load(sys.stdin)["total_detections"])' 2>/dev/null
}

# Highest applied schema version, read from the database file.
schema_version() {
    python3 - "$1" <<'PY'
import sqlite3, sys
c = sqlite3.connect(f"file:{sys.argv[1]}?mode=ro", uri=True)
print(c.execute("SELECT MAX(version) FROM schema_version").fetchone()[0])
PY
}

# Whether, within $1 seconds, systemd reports the unit active and the
# dashboard answers.
up_within() {
    for _ in $(seq 1 "$1"); do
        if systemctl is-active --quiet "${SERVICE}" \
            && curl -fsS -o /dev/null "http://${LISTEN}/api/v2/health" 2>/dev/null; then
            return 0
        fi
        sleep 1
    done
    return 1
}

wait_up() {
    up_within 300 \
        || die "the service did not come up within 300 s (is-active: $(systemctl is-active "${SERVICE}"))"
}

# Releases whose own installer enables the geomodel, whose binary then gets
# the service killed under its own unit on x86_64 (SIGSYS: ONNX Runtime's
# default thread pool calls sched_setaffinity, which the unit denied). Such a
# station is started here with a drop-in allowing that call, and the drop-in is
# removed before the update, so the candidate is judged on its own unit with
# the geomodel still configured. Named, so a release that fails to start for
# any other reason still fails this test.
KNOWN_GEOMODEL_SIGSYS="0.16.0 0.16.1 0.17.0"
KNOWN_DROPIN="/etc/systemd/system/${SERVICE}.service.d/zz-upgrade-e2e-known-sigsys.conf"

# Whether the geomodel loads within 60 s of the service's current start. Every
# release since 0.15.0 opens its load message with this phrase. The dashboard
# answers before the daemon has loaded it, so this waits rather than looks once.
geomodel_loaded() {
    local since log
    since="$(systemctl show -p ExecMainStartTimestamp --value "${SERVICE}")"
    for _ in $(seq 1 60); do
        log="$(journalctl -u "${SERVICE}" --since "${since}" --no-pager -o cat 2>/dev/null)"
        grep -q 'metadata model loaded' <<<"${log}" && return 0
        sleep 1
    done
    return 1
}

# Hand the running daemon one recording and wait for the count to rise.
# The unit runs with PrivateTmp, so the watch directory it sees is reached
# through its own mount view rather than the host's /tmp.
record_one() {
    local before="$1" pid dir name now
    pid="$(systemctl show -p MainPID --value "${SERVICE}")"
    [ -n "${pid}" ] && [ "${pid}" != 0 ] || die "no main PID to hand a recording to"
    dir="/proc/${pid}/root/tmp/birdnet-stream"
    [ -d "${dir}" ] || die "the service has no watch directory at ${dir}"
    name="$(date '+%Y-%m-%d-birdnet-%H:%M:%S').wav"
    cp "${FIXTURE}" "${dir}/${name}.part"
    chown "${SERVICE_USER}:" "${dir}/${name}.part"
    mv "${dir}/${name}.part" "${dir}/${name}"
    # One recording yields several detections, written one after another, so
    # wait for the count to rise and then hold for 5 s. Returning at the first
    # rise left a row arriving after the snapshot, which then read as a row
    # appearing across the next step.
    local last="" steady=0
    for _ in $(seq 1 180); do
        now="$(detections)"
        if [ -n "${now}" ] && [ "${now}" -gt "${before}" ]; then
            if [ "${now}" = "${last}" ]; then
                steady=$((steady + 1))
                [ "${steady}" -ge 5 ] && { echo "${now}"; return 0; }
            else
                steady=0
            fi
            last="${now}"
        fi
        sleep 1
    done
    die "a 30 s magpie recording produced no detection within 180 s (count stayed ${before})"
}

running_version() { "${BIN}" --version | awk '{print $2}'; }

# ── 0. the previous release, from its own published assets ─────────────────

STEP="fetch ${PREV_VERSION}"
PREV_DIR="${PREV_DIR:-}"
if [ -z "${PREV_DIR}" ]; then
    PREV_DIR="${WORK}/prev"
    mkdir -p "${PREV_DIR}"
    base="https://github.com/${REPO}/releases/download/v${PREV_VERSION}"
    for f in install.sh SHA256SUMS "birdnet-behavior-${PREV_VERSION}-${ARCH}.tar.gz"; do
        curl -fsSL --retry 4 -o "${PREV_DIR}/${f}" "${base}/${f}" || die "could not download ${base}/${f}"
    done
    (cd "${PREV_DIR}" && sha256sum --ignore-missing -c SHA256SUMS) || die "v${PREV_VERSION} assets fail their SHA256SUMS"
fi
PREV_TARBALL="${PREV_DIR}/birdnet-behavior-${PREV_VERSION}-${ARCH}.tar.gz"
[ -f "${PREV_DIR}/install.sh" ] && [ -f "${PREV_TARBALL}" ] || die "PREV_DIR lacks install.sh or ${PREV_TARBALL##*/}"

id "${SERVICE_USER}" >/dev/null 2>&1 || useradd -m "${SERVICE_USER}"
HOME_DIR="$(getent passwd "${SERVICE_USER}" | cut -d: -f6)"
DATA_DIR="${HOME_DIR}/BirdNet-Behavior"
DB="${DATA_DIR}/birds.db"

if [ -n "${MODEL_CACHE}" ] && [ -n "$(ls -A "${MODEL_CACHE}" 2>/dev/null)" ]; then
    mkdir -p "${DATA_DIR}/models"
    cp -a "${MODEL_CACHE}/." "${DATA_DIR}/models/"
    chown -R "${SERVICE_USER}:" "${DATA_DIR}"
    echo "staged $(ls "${DATA_DIR}/models" | wc -l) model file(s) from ${MODEL_CACHE}"
fi

# ── 1. install the previous release with its own installer ──────────────────

log "1. install v${PREV_VERSION} with its own install.sh"
STEP="install ${PREV_VERSION}"
SUDO_USER="${SERVICE_USER}" BIRDNET_NONINTERACTIVE=1 BIRDNET_LISTEN="${LISTEN}" \
    BIRDNET_BINARY_TARBALL="${PREV_TARBALL}" VERSION="${PREV_VERSION}" \
    bash "${PREV_DIR}/install.sh" install --version "${PREV_VERSION}" --noninteractive \
    > "${WORK}/install-prev.log" 2>&1 \
    || { tail -40 "${WORK}/install-prev.log"; die "v${PREV_VERSION} install.sh exited non-zero"; }
systemctl is-active --quiet "${SERVICE}" || systemctl start "${SERVICE}"
if ! up_within 90 \
    && [ "$(systemctl show -p ExecMainStatus --value "${SERVICE}")" = 31 ] \
    && grep -qwF -- "${PREV_VERSION}" <<<"${KNOWN_GEOMODEL_SIGSYS}" \
    && grep -q '^METADATA_' /etc/birdnet/birdnet.conf; then
    echo "  KNOWN  v${PREV_VERSION} is killed by SIGSYS with the geomodel on; starting it with sched_setaffinity allowed"
    mkdir -p "$(dirname "${KNOWN_DROPIN}")"
    printf '[Service]\nSystemCallFilter=sched_setaffinity\n' > "${KNOWN_DROPIN}"
    systemctl daemon-reload
    systemctl restart "${SERVICE}"
fi
wait_up
[ "$(running_version)" = "${PREV_VERSION}" ] || die "installed binary is $(running_version), not ${PREV_VERSION}"
pass "v${PREV_VERSION} installed and serving"

if [ -n "${MODEL_CACHE}" ] && [ -z "$(ls -A "${MODEL_CACHE}" 2>/dev/null)" ]; then
    mkdir -p "${MODEL_CACHE}"
    cp -a "${DATA_DIR}/models/." "${MODEL_CACHE}/"
fi

# ── 2. the previous binary records detections of its own ────────────────────

log "2. v${PREV_VERSION} records detections"
STEP="seed on ${PREV_VERSION}"
n0="$(detections)"; [ -n "${n0}" ] || die "/api/v2/stats did not answer"
n1="$(record_one "${n0}")" || exit 1
n1="$(record_one "${n1}")" || exit 1
PREV_SCHEMA="$(schema_version "${DB}")" || die "could not read the schema version"
pass "v${PREV_VERSION} holds ${n1} detection(s) at schema ${PREV_SCHEMA}"

# ── 3. upgrade in place, service running, as the operator does ──────────────

log "3. update to v${CAND_VERSION} with the candidate install.sh"
STEP="update to ${CAND_VERSION}"
# The candidate is judged on the unit its own installer writes, not on the
# allowance step 1 may have added for a known-broken previous release.
if [ -e "${KNOWN_DROPIN}" ]; then
    rm -f "${KNOWN_DROPIN}"
    systemctl daemon-reload
fi
SUDO_USER="${SERVICE_USER}" BIRDNET_NONINTERACTIVE=1 \
    BIRDNET_BINARY_TARBALL="$(realpath "${CANDIDATE_TARBALL}")" VERSION="${CAND_VERSION}" \
    bash "${CANDIDATE_INSTALLER}" update --version "${CAND_VERSION}" --noninteractive \
    > "${WORK}/install-candidate.log" 2>&1 \
    || { tail -40 "${WORK}/install-candidate.log"; die "candidate install.sh update exited non-zero"; }

# ── 4. the upgraded station ─────────────────────────────────────────────────

log "4. the upgraded station"
STEP="after update"
wait_up
[ "$(running_version)" = "${CAND_VERSION}" ] || die "binary after update is $(running_version), not ${CAND_VERSION}"
pass "active on v${CAND_VERSION}"
n2="$(detections)"
[ "${n2}" = "${n1}" ] || die "detections went from ${n1} to ${n2} across the upgrade"
pass "all ${n2} detection(s) survived the upgrade"
CAND_SCHEMA="$(schema_version "${DB}")" || die "could not read the schema version"
[ "${CAND_SCHEMA}" -ge "${PREV_SCHEMA}" ] || die "schema went backwards: ${PREV_SCHEMA} → ${CAND_SCHEMA}"
pass "schema ${PREV_SCHEMA} → ${CAND_SCHEMA}"
grep -q '^METADATA_MODEL_PATH=' /etc/birdnet/birdnet.conf \
    || die "the config has no geomodel after the update, so this run cannot speak for it"
geomodel_loaded || die "the geomodel is configured but did not load on v${CAND_VERSION}"
pass "the geomodel is configured and loaded"
n3="$(record_one "${n2}")" || exit 1
pass "v${CAND_VERSION} records a new detection (${n2} → ${n3})"

# ── 5. the rollback the installer printed, verbatim ─────────────────────────

log "5. roll back with the command the update printed"
STEP="rollback"
hint="$(sed -E 's/\x1b\[[0-9;]*m//g' "${WORK}/install-candidate.log" \
    | grep -o 'roll back with: .*' | tail -1 | sed 's/^roll back with: //')"
[ -n "${hint}" ] || die "the update printed no rollback command"
echo "  running: ${hint}"
# The test already runs as root; sudo may be absent on a minimal host.
bash -c "${hint//sudo /}" || die "the printed rollback command failed"

# ── 6. the rolled-back station ──────────────────────────────────────────────

log "6. the rolled-back station"
STEP="after rollback"
wait_up
[ "$(running_version)" = "${PREV_VERSION}" ] || die "binary after rollback is $(running_version), not ${PREV_VERSION}"
pass "active on v${PREV_VERSION} again"
geomodel_loaded || die "the geomodel is configured but did not load on the rolled-back v${PREV_VERSION}"
pass "the geomodel loaded on v${PREV_VERSION}, under the candidate's unit"
n4="$(detections)"
[ "${n4}" = "${n3}" ] || die "detections went from ${n3} to ${n4} across the rollback"
pass "all ${n4} detection(s) survived the rollback"
n5="$(record_one "${n4}")" || exit 1
pass "v${PREV_VERSION} records a new detection on schema ${CAND_SCHEMA} (${n4} → ${n5})"

echo
echo "upgrade-e2e PASSED: v${PREV_VERSION} → v${CAND_VERSION} → rollback to v${PREV_VERSION}"
rm -rf "${WORK}"
