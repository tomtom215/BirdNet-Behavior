#!/bin/sh
# =============================================================================
# BirdNet-Behavior container entrypoint
#
# Responsibilities:
#   1. Auto-download the BirdNET+ V3.0 model from Zenodo (first run only)
#      with resume support, progress logging every 15 s, and clear failure
#      diagnostics.
#   2. Set container-appropriate defaults (listen on all interfaces).
#   3. Exec the birdnet-behavior binary, forwarding any extra arguments.
#
# Environment variables:
#   BIRDNET_MODEL_DIR           Model directory (default: /data/model)
#   BIRDNET_SKIP_MODEL_DOWNLOAD Set to "1" to skip auto-download
#   BIRDNET_MODEL               Path to ONNX model file (auto-set if blank)
#   BIRDNET_LABELS              Path to labels CSV file (auto-set if blank)
#   BIRDNET_LISTEN              Web server address (default: 0.0.0.0:8502)
#
# All other BIRDNET_* variables are passed through to the binary unchanged.
# =============================================================================
set -eu

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
log()   { printf '[birdnet] %s\n' "$*"; }
warn()  { printf '[birdnet] WARNING: %s\n' "$*" >&2; }
die()   { printf '[birdnet] ERROR: %s\n' "$*" >&2; exit 1; }
rule()  { log '----------------------------------------------------------------'; }

# Format a byte count as a short, human-readable string (541 MB, 809 KB, …).
# Falls back gracefully if numfmt is missing.
human_bytes() {
    b="${1:-0}"
    if command -v numfmt >/dev/null 2>&1; then
        numfmt --to=si --suffix=B --format='%.1f' -- "$b" 2>/dev/null \
            || printf '%s B\n' "$b"
    else
        awk -v b="$b" 'BEGIN {
            split("B KB MB GB TB", u, " "); i=1;
            while (b >= 1000 && i < 5) { b = b / 1000; i++ }
            if (i == 1) { printf "%d %s\n", b, u[i] }
            else        { printf "%.1f %s\n", b, u[i] }
        }'
    fi
}

# Fetch the Content-Length header for a URL; prints "0" on failure.
remote_size() {
    url="$1"
    curl --silent --location --head --max-time 15 --retry 2 --retry-delay 3 "$url" 2>/dev/null \
        | awk 'BEGIN{IGNORECASE=1; v=0}
               /^content-length:/ {gsub(/\r/,""); v=$2}
               END{print v+0}'
}

# ---------------------------------------------------------------------------
# Model paths
# ---------------------------------------------------------------------------
MODEL_DIR="${BIRDNET_MODEL_DIR:-/data/model}"
MODEL_FILE="BirdNET+_V3.0-preview3_Global_11K_FP32.onnx"
LABELS_FILE="BirdNET+_V3.0-preview3_Global_11K_Labels.csv"
ZENODO_BASE="https://zenodo.org/api/records/18247420/files"

# Respect explicit overrides; otherwise use the default paths under MODEL_DIR.
: "${BIRDNET_MODEL:=${MODEL_DIR}/${MODEL_FILE}}"
: "${BIRDNET_LABELS:=${MODEL_DIR}/${LABELS_FILE}}"
export BIRDNET_MODEL BIRDNET_LABELS

# ---------------------------------------------------------------------------
# Download with resume + periodic progress logging
# ---------------------------------------------------------------------------
#
# curl runs in the background so a companion loop can print a single log line
# every 15 s (readable in `docker compose logs`, unlike the raw \r-based
# progress meter which is unreadable in non-TTY log streams).
#
# Partial downloads are resumed via `--continue-at -` so that a lost
# connection on a 500 MB fetch does not force the user to start over.
# ---------------------------------------------------------------------------
download_if_missing() {
    dest="$1"
    url="$2"
    desc="$3"

    if [ -f "$dest" ]; then
        actual="$(wc -c < "$dest" 2>/dev/null || echo 0)"
        log "${desc}: already cached ($(human_bytes "$actual")) — skipping download."
        return 0
    fi

    # Pre-flight: how big is the file on the server?
    total="$(remote_size "$url")"
    if [ "$total" -gt 0 ] 2>/dev/null; then
        total_h="$(human_bytes "$total")"
    else
        total=0
        total_h="unknown size"
    fi

    rule
    log "Downloading ${desc} (${total_h})"
    log "  from:  ${url}"
    log "  to:    ${dest}"
    log "  This runs only on first start. The model is cached in the"
    log "  Docker volume so subsequent container starts are instant."
    log "  Typical download: 1–3 min on fibre, 5–15 min on home broadband."
    rule

    tmpfile="${dest}.tmp"
    start_ts="$(date +%s)"

    # Background curl with resume support.
    curl --fail --location --silent --show-error \
         --retry 5 --retry-delay 10 --retry-max-time 900 \
         --connect-timeout 30 \
         --continue-at - \
         --output "${tmpfile}" \
         "${url}" &
    curl_pid=$!

    # Progress watcher — logs one line every 15 s while curl is still running.
    (
        while kill -0 "${curl_pid}" 2>/dev/null; do
            sleep 15
            kill -0 "${curl_pid}" 2>/dev/null || break
            got=0
            [ -f "${tmpfile}" ] && got="$(wc -c < "${tmpfile}" 2>/dev/null || echo 0)"
            elapsed=$(( $(date +%s) - start_ts ))
            if [ "${total}" -gt 0 ] 2>/dev/null && [ "${got}" -gt 0 ] 2>/dev/null; then
                pct=$(( got * 100 / total ))
                # crude speed average
                if [ "${elapsed}" -gt 0 ] 2>/dev/null; then
                    speed=$(( got / elapsed ))
                    log "  …${desc}: ${pct}%  ($(human_bytes "${got}") / ${total_h}, $(human_bytes "${speed}")/s, ${elapsed}s elapsed)"
                else
                    log "  …${desc}: ${pct}%  ($(human_bytes "${got}") / ${total_h})"
                fi
            else
                log "  …${desc}: $(human_bytes "${got}") so far (${elapsed}s elapsed)"
            fi
        done
    ) &
    watcher_pid=$!

    # Wait for curl; preserve its exit code.
    set +e
    wait "${curl_pid}"
    rc=$?
    set -e
    kill "${watcher_pid}" 2>/dev/null || true
    wait "${watcher_pid}" 2>/dev/null || true

    elapsed=$(( $(date +%s) - start_ts ))

    if [ "${rc}" -eq 0 ]; then
        mv "${tmpfile}" "${dest}"
        final="$(wc -c < "${dest}" 2>/dev/null || echo 0)"
        log "  done: ${desc} saved ($(human_bytes "${final}") in ${elapsed}s)"
        return 0
    fi

    # Failure path — keep the partial file so the next restart can resume.
    partial=0
    [ -f "${tmpfile}" ] && partial="$(wc -c < "${tmpfile}" 2>/dev/null || echo 0)"
    warn "${desc}: curl exited ${rc} after ${elapsed}s"
    if [ "${partial}" -gt 0 ] 2>/dev/null; then
        warn "Partial file ($(human_bytes "${partial}")) kept at ${tmpfile}."
        warn "The next container start will resume from where it left off."
    fi
    warn "Common causes:"
    warn "  • no internet in the container (check the host's DNS/firewall)"
    warn "  • Zenodo is temporarily unreachable (retry in a few minutes)"
    warn "  • the volume is out of disk (df -h on the host's docker root)"
    die "Failed to download ${desc} from ${url}"
}

# ---------------------------------------------------------------------------
# Model auto-download driver
# ---------------------------------------------------------------------------
if [ "${BIRDNET_SKIP_MODEL_DOWNLOAD:-}" = "1" ]; then
    log "BIRDNET_SKIP_MODEL_DOWNLOAD=1 — skipping model download."
    [ -f "${BIRDNET_MODEL}"  ] || warn "Model not found at ${BIRDNET_MODEL}"
    [ -f "${BIRDNET_LABELS}" ] || warn "Labels not found at ${BIRDNET_LABELS}"
else
    mkdir -p "${MODEL_DIR}"

    # Announce the model source once, up front, so users know what's happening
    # even if the model is already cached and no download is needed.
    log "BirdNET+ V3.0 model directory: ${MODEL_DIR}"

    download_if_missing \
        "${BIRDNET_MODEL}" \
        "${ZENODO_BASE}/${MODEL_FILE}/content" \
        "BirdNET+ V3.0 model (ONNX)"

    download_if_missing \
        "${BIRDNET_LABELS}" \
        "${ZENODO_BASE}/${LABELS_FILE}/content" \
        "species labels CSV"

    log "Model ready."
fi

# ---------------------------------------------------------------------------
# Container defaults
# ---------------------------------------------------------------------------
# The binary defaults to 127.0.0.1:8502 (loopback only), which is unreachable
# from outside the container.  Override to bind on all interfaces unless the
# user has already set BIRDNET_LISTEN explicitly.
: "${BIRDNET_LISTEN:=0.0.0.0:8502}"
export BIRDNET_LISTEN

# ---------------------------------------------------------------------------
# Audio source check (advisory only — does not prevent startup)
# ---------------------------------------------------------------------------
if [ -z "${BIRDNET_ALSA_DEVICE:-}" ] \
    && [ -z "${BIRDNET_PIPEWIRE_DEVICE:-}" ] \
    && [ -z "${BIRDNET_RTSP_URL:-}" ] \
    && [ -z "${BIRDNET_RTSP_URLS:-}" ]; then
    warn "No audio source configured."
    warn "Set one of: BIRDNET_ALSA_DEVICE, BIRDNET_PIPEWIRE_DEVICE,"
    warn "            BIRDNET_RTSP_URL, or BIRDNET_RTSP_URLS."
    warn "The web UI will start but no detections will be produced."
    warn "File-watch mode: drop WAV files into \$BIRDNET_WATCH_DIR instead."
fi

# ---------------------------------------------------------------------------
# Launch
# ---------------------------------------------------------------------------
rule
log "Starting birdnet-behavior  (listen: ${BIRDNET_LISTEN})"
rule
exec /usr/local/bin/birdnet-behavior "$@"
