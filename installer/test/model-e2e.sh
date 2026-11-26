#!/usr/bin/env bash
# installer/test/model-e2e.sh — does a real install get the real models, detect a
# real bird with them, and look after them when they are damaged? (MDL-18)
#
# Every other model test stubs something: the installer tests stub the binary,
# the station-harness tests run fixture models, and the download tests run
# against a loopback origin. None of them installed a station the way an owner
# does and let it fetch the files it will run. This does, with nothing stubbed,
# on a real systemd host:
#
#   1. install.sh installs the freshly built binary (CANDIDATE_TARBALL) as a
#      service, and the binary downloads the real default models (BirdNET V2.4
#      and its range filter) from their real origins
#   2. the station, placed in Berlin, is handed the magpie recording
#      (tests/testdata/Pica_pica_30s.wav) and must store a Pica pica detection,
#      with the model state the healthy one: the detector running, the
#      classifier judging, the filter active, every model file verified
#   3. each model file in turn is damaged while the station runs (eight bytes
#      overwritten, the size kept, so only a hash finds it): the station must
#      say so and fetch it again, and keep running, since what it loaded is
#      still in memory
#   4. each model file in turn is damaged and the service restarted, as a
#      reboot would: the station must say so, fetch it again, and come back
#      healthy and detecting — restarting itself once where it started without
#      the file
#
# The container image, with its own entrypoint, is not exercised here: that
# half is still to be written (MDL-18 stays open for it).
#
# It changes the machine it runs on: it installs a service, creates a user and
# writes under /usr/local, /etc and /home. Run it on a disposable host (CI).
#
# Usage (as root, on a host where systemd is PID 1):
#   CANDIDATE_TARBALL=dist/birdnet-behavior-0.19.0-x86_64-unknown-linux-gnu.tar.gz \
#   installer/test/model-e2e.sh
#
# Optional:
#   BIRDNET_MODEL_MIRROR  a mirror of the model files, tried before the
#                         catalogue's origins (for a host with no internet;
#                         the scheduled run leaves it unset, so the real
#                         origins are what is tested)
#   SERVICE_USER          account the station runs as (default: bnbmodels)

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SERVICE="birdnet-behavior"
BIN="/usr/local/bin/birdnet-behavior"
LISTEN="127.0.0.1:8503"
FIXTURE="${REPO_ROOT}/tests/testdata/Pica_pica_30s.wav"
CONFIG="/etc/birdnet/birdnet.conf"
# Berlin: the range filter keeps the magpie here all year.
STATION_LAT="52.5200"
STATION_LON="13.4050"
# How often the running station looks at its model files again, set for this
# test with a drop-in (the default is an hour).
REPAIR_INTERVAL=5

: "${CANDIDATE_TARBALL:?set CANDIDATE_TARBALL to the candidate release tarball}"
SERVICE_USER="${SERVICE_USER:-bnbmodels}"
DROPIN="/etc/systemd/system/${SERVICE}.service.d/zz-model-e2e.conf"
CA_COPY="/etc/ssl/model-e2e-ca.pem"

STEP="setup"
log()  { printf '\n=== %s\n' "$*"; }
pass() { printf '  PASS  %s\n' "$*"; }
die() {
    {
        printf '  FAIL  [%s] %s\n' "${STEP}" "$*"
        echo "--- systemctl status"
        systemctl status "${SERVICE}" --no-pager -l 2>&1 | tail -30
        echo "--- journal (last 150 lines)"
        journalctl -u "${SERVICE}" --no-pager -n 150 2>&1
    } >&2
    exit 1
}

[ "$(id -u)" -eq 0 ] || { echo "model-e2e: must run as root"; exit 2; }
[ -d /run/systemd/system ] || { echo "model-e2e: systemd is not running here; this test needs it"; exit 2; }
[ -f "${CANDIDATE_TARBALL}" ] || { echo "model-e2e: no tarball at ${CANDIDATE_TARBALL}"; exit 2; }
[ -f "${FIXTURE}" ] || { echo "model-e2e: missing fixture ${FIXTURE}"; exit 2; }
if [ -e "${BIN}" ] || [ -e "/etc/systemd/system/${SERVICE}.service" ]; then
    echo "model-e2e: a station is already installed here; this test needs a clean host"
    exit 2
fi
CAND_VERSION="$(basename "${CANDIDATE_TARBALL}" | sed -E 's/^birdnet-behavior-([0-9][^-]*)-.*/\1/')"
WORK="$(mktemp -d /var/tmp/model-e2e.XXXXXX)"

# ── helpers ──────────────────────────────────────────────────────────────────

metrics() { curl -fsS "http://${LISTEN}/api/v2/metrics" 2>/dev/null; }

# The value of one series, exactly as exposed; empty when it is not there.
series() {
    local body
    body="$(metrics)" || return 1
    awk -v s="$1" '$1 == s { print $2; exit }' <<<"${body}"
}

# Whether, within $2 seconds, series $1 reads 1. Tolerates a station that is
# restarting and does not answer.
state_within() {
    for _ in $(seq 1 "$2"); do
        [ "$(series "$1" 2>/dev/null)" = 1 ] && return 0
        sleep 1
    done
    return 1
}

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

main_pid() { systemctl show -p MainPID --value "${SERVICE}"; }

# Journal lines of the service since $1 that contain $2.
journal_has() {
    local text
    text="$(journalctl -u "${SERVICE}" --since "$1" --no-pager -o cat 2>/dev/null)"
    grep -qF -- "$2" <<<"${text}"
}

magpies() {
    python3 - "${DB}" <<'PY'
import sqlite3, sys
c = sqlite3.connect(f"file:{sys.argv[1]}?mode=ro", uri=True)
print(c.execute("SELECT COUNT(*) FROM detections WHERE Sci_Name = 'Pica pica'").fetchone()[0])
PY
}

# Hand the running daemon the magpie and wait for a new magpie row.
record_magpie() {
    local before pid dir name
    before="$(magpies)" || die "could not count magpie rows"
    pid="$(main_pid)"
    [ -n "${pid}" ] && [ "${pid}" != 0 ] || die "no main PID to hand a recording to"
    dir="/proc/${pid}/root/tmp/birdnet-stream"
    [ -d "${dir}" ] || die "the service has no watch directory at ${dir}"
    name="$(date '+%Y-%m-%d-birdnet-%H:%M:%S').wav"
    cp "${FIXTURE}" "${dir}/${name}.part"
    chown "${SERVICE_USER}:" "${dir}/${name}.part"
    mv "${dir}/${name}.part" "${dir}/${name}"
    for _ in $(seq 1 180); do
        [ "$(magpies)" -gt "${before}" ] && return 0
        sleep 1
    done
    die "a 30 s magpie recording stored no Pica pica within 180 s (magpie rows stayed ${before})"
}

# The model state an owner set the station up for.
healthy() {
    local f
    state_within 'birdnet_detector_state{state="running"}' 180 \
        || die "the detector is not running ($(series 'birdnet_detector_state{state="failed"}' 2>/dev/null) failed)"
    state_within 'birdnet_classifier_state{classifier="birdnet",state="judging"}' 30 \
        || die "the classifier is not judging"
    state_within 'birdnet_occurrence_filter_state{state="active"}' 30 \
        || die "the occurrence filter is not active"
    for f in "${FILES[@]}"; do
        state_within "birdnet_model_file_state{file=\"${f}\",state=\"verified\"}" 120 \
            || state_within "birdnet_model_file_state{file=\"${f}\",state=\"repaired\"}" 1 \
            || die "${f} is not reported as the file this build pins"
    done
}

# The sha256 the catalogue pins for the model file $1 (`<id>.onnx`), read
# with the reader the release workflow uses, so this holds the station to the
# pins it was built with.
pinned_sha() {
    local sha
    sha="$(bash "${REPO_ROOT}/installer/catalogue-field.sh" "${1%.onnx}" model_sha256)"
    [ -n "${sha}" ] && echo "${sha}"
}

# Overwrite eight bytes in the middle of $1, keeping its size.
damage() {
    local file="$1" size
    size="$(stat -c %s "${file}")"
    printf 'DAMAGED!' | dd of="${file}" bs=1 seek=$((size / 2)) conv=notrunc status=none
}

# ── 1. install, letting the binary fetch the real models ────────────────────

log "1. install v${CAND_VERSION} with install.sh; the binary downloads the models"
STEP="install"
id "${SERVICE_USER}" >/dev/null 2>&1 || useradd -m "${SERVICE_USER}"
HOME_DIR="$(getent passwd "${SERVICE_USER}" | cut -d: -f6)"
DATA_DIR="${HOME_DIR}/BirdNet-Behavior"
MODEL_DIR="${DATA_DIR}/models"
DB="${DATA_DIR}/birds.db"
SUDO_USER="${SERVICE_USER}" BIRDNET_NONINTERACTIVE=1 BIRDNET_LISTEN="${LISTEN}" \
    BIRDNET_BINARY_TARBALL="$(realpath "${CANDIDATE_TARBALL}")" VERSION="${CAND_VERSION}" \
    bash "${REPO_ROOT}/install.sh" install --version "${CAND_VERSION}" --noninteractive \
    > "${WORK}/install.log" 2>&1 \
    || { tail -60 "${WORK}/install.log"; die "install.sh exited non-zero"; }
grep -q '^MODEL_PATH=' "${CONFIG}" || die "the config names no classifier"
grep -q '^METADATA_MODEL_PATH=' "${CONFIG}" || die "the config names no occurrence filter: the install did not get it"
FILES=(birdnet-v24.onnx birdnet-v24_labels.txt birdnet-v24-range-v2.onnx)
for f in "${FILES[@]}"; do
    [ -s "${MODEL_DIR}/${f}" ] || die "the install left no ${f} in ${MODEL_DIR}"
done
for f in birdnet-v24.onnx birdnet-v24-range-v2.onnx; do
    want="$(pinned_sha "${f}")" || die "no pin for ${f} in the catalogue"
    got="$(sha256sum "${MODEL_DIR}/${f}" | cut -d' ' -f1)"
    [ "${got}" = "${want}" ] || die "${f} hashes to ${got}, not the pinned ${want}"
done
pass "install.sh installed the binary and the real V2.4 and range filter, each the pinned file"

# A station with a place, and a repair interval a test can wait out.
printf 'LATITUDE=%s\nLONGITUDE=%s\n' "${STATION_LAT}" "${STATION_LON}" >> "${CONFIG}"
mkdir -p "$(dirname "${DROPIN}")"
printf '[Service]\nEnvironment=BIRDNET_MODEL_REPAIR_INTERVAL=%s\n' "${REPAIR_INTERVAL}" > "${DROPIN}"
if [ -n "${BIRDNET_MODEL_MIRROR:-}" ]; then
    printf 'Environment=BIRDNET_MODEL_MIRROR=%s\n' "${BIRDNET_MODEL_MIRROR}" >> "${DROPIN}"
fi
# A host that reaches the internet through a proxy: the service fetches its
# models again itself, so it needs the proxy the installer's download used.
# The CA bundle is copied out of any home directory, which the unit hides.
for v in HTTPS_PROXY https_proxy NO_PROXY no_proxy; do
    [ -n "${!v:-}" ] && printf 'Environment=%s=%s\n' "${v}" "${!v}" >> "${DROPIN}"
done
if [ -n "${SSL_CERT_FILE:-}" ] && [ -r "${SSL_CERT_FILE}" ]; then
    cp "${SSL_CERT_FILE}" "${CA_COPY}"
    chmod 0644 "${CA_COPY}"
    printf 'Environment=SSL_CERT_FILE=%s\n' "${CA_COPY}" >> "${DROPIN}"
fi
systemctl daemon-reload
systemctl restart "${SERVICE}"
up_within 180 || die "the service did not come up within 180 s"

# ── 2. the magpie, and the healthy state ────────────────────────────────────

log "2. the station detects the magpie and reports the healthy model state"
STEP="detect"
healthy
record_magpie
pass "a Pica pica detection is stored; detector running, classifier judging, filter active, files verified"

# ── 3. damaged while running: found at the interval, fetched, no restart ────

for f in "${FILES[@]}"; do
    log "3. ${f} damaged while the station runs"
    STEP="damage ${f} while running"
    path="${MODEL_DIR}/${f}"
    before_sum="$(sha256sum "${path}" | cut -d' ' -f1)"
    pid="$(main_pid)"
    since="$(date '+%Y-%m-%d %H:%M:%S')"
    damage "${path}"
    [ "$(sha256sum "${path}" | cut -d' ' -f1)" != "${before_sum}" ] || die "the damage did not change ${f}"
    for _ in $(seq 1 180); do
        [ "$(sha256sum "${path}" | cut -d' ' -f1)" = "${before_sum}" ] && break
        sleep 1
    done
    [ "$(sha256sum "${path}" | cut -d' ' -f1)" = "${before_sum}" ] \
        || die "${f} was not fetched again within 180 s"
    journal_has "${since}" "missing or damaged" || die "the station did not say ${f} was damaged"
    state_within "birdnet_model_file_state{file=\"${f}\",state=\"repaired\"}" 30 \
        || die "the station does not report ${f} as repaired"
    [ "$(main_pid)" = "${pid}" ] || die "the station restarted for a file it had loaded (pid ${pid} → $(main_pid))"
    healthy
    pass "${f}: said, fetched again, verified, and the running station was not restarted"
done
record_magpie
pass "still detecting the magpie"

# ── 4. damaged at a start: fetched, restarted once where it was without it ──

for f in "${FILES[@]}"; do
    log "4. ${f} damaged, then the service restarted"
    STEP="damage ${f} at start"
    path="${MODEL_DIR}/${f}"
    before_sum="$(sha256sum "${path}" | cut -d' ' -f1)"
    systemctl stop "${SERVICE}"
    damage "${path}"
    since="$(date '+%Y-%m-%d %H:%M:%S')"
    systemctl start "${SERVICE}"
    up_within 180 || die "the service did not come up"
    for _ in $(seq 1 240); do
        [ "$(sha256sum "${path}" | cut -d' ' -f1)" = "${before_sum}" ] && break
        sleep 1
    done
    [ "$(sha256sum "${path}" | cut -d' ' -f1)" = "${before_sum}" ] \
        || die "${f} was not fetched again within 240 s of the start"
    journal_has "${since}" "missing or damaged" || die "the station did not say ${f} was damaged"
    up_within 180 || die "the service is not up after the repair"
    healthy
    record_magpie
    # Counted from systemd's own line: the binary's start-up line is also
    # written by the unit's preflight doctor. Two: the start above, and the one
    # the station asked for once the file it loaded damaged verified again.
    sleep $((REPAIR_INTERVAL * 3))
    starts="$(journalctl -u "${SERVICE}" --since "${since}" --no-pager -o cat | grep -c "^Started ${SERVICE}.service")"
    [ "${starts}" = 2 ] || die "the service started ${starts} time(s) after ${f} was damaged; expected 2 (no restart, or a loop)"
    pass "${f}: said, fetched again, restarted once to load it, healthy, and detecting the magpie"
done

rm -f "${DROPIN}" "${CA_COPY}"
systemctl daemon-reload
echo
echo "model-e2e PASSED: v${CAND_VERSION} installed, fetched, detected and repaired its real models"
rm -rf "${WORK}"
