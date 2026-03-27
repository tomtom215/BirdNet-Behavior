#!/bin/sh
# =============================================================================
# BirdNet-Behavior container entrypoint
#
# Responsibilities:
#   1. Auto-download the BirdNET+ V3.0 model from Zenodo (first run only)
#   2. Set container-appropriate defaults (listen on all interfaces)
#   3. Exec the birdnet-behavior binary, forwarding any extra arguments
#
# Environment variables:
#   BIRDNET_MODEL_DIR           Model directory (default: /data/model)
#   BIRDNET_SKIP_MODEL_DOWNLOAD Set to "1" to skip auto-download (bring your own model)
#   BIRDNET_MODEL               Path to ONNX model file (auto-set if not provided)
#   BIRDNET_LABELS              Path to labels CSV file (auto-set if not provided)
#   BIRDNET_LISTEN              Web server address (default: 0.0.0.0:8502)
#
# All other BIRDNET_* variables are passed through to the binary unchanged.
# =============================================================================
set -e

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
log()  { printf '[birdnet] %s\n' "$*"; }
warn() { printf '[birdnet] WARNING: %s\n' "$*" >&2; }
die()  { printf '[birdnet] ERROR: %s\n' "$*" >&2; exit 1; }

# ---------------------------------------------------------------------------
# Model paths
# ---------------------------------------------------------------------------
MODEL_DIR="${BIRDNET_MODEL_DIR:-/data/model}"
MODEL_FILE="BirdNET+_V3.0-preview3_Global_11K_FP32.onnx"
LABELS_FILE="BirdNET+_V3.0-preview3_Global_11K_Labels.csv"
ZENODO_BASE="https://zenodo.org/api/records/18247420/files"

# Respect explicit overrides; otherwise use the default paths under MODEL_DIR
: "${BIRDNET_MODEL:=${MODEL_DIR}/${MODEL_FILE}}"
: "${BIRDNET_LABELS:=${MODEL_DIR}/${LABELS_FILE}}"
export BIRDNET_MODEL BIRDNET_LABELS

# ---------------------------------------------------------------------------
# Model auto-download
# ---------------------------------------------------------------------------
download_if_missing() {
    local dest="$1"
    local url="$2"
    local desc="$3"

    if [ -f "$dest" ]; then
        return 0
    fi

    log "Downloading ${desc}…"
    log "  → ${dest}"

    # curl flags:
    #   --fail         treat HTTP errors as fatal
    #   --location     follow redirects (Zenodo uses CDN redirects)
    #   --retry 3      retry transient failures
    #   --retry-delay 5  wait 5 s between retries
    #   --progress-bar show a simple progress indicator
    #   -o <tmp>       write to temp file, rename on success
    curl --fail --location \
         --retry 3 --retry-delay 5 \
         --progress-bar \
         -o "${dest}.tmp" \
         "${url}" \
    || die "Failed to download ${desc} from ${url}"

    mv "${dest}.tmp" "${dest}"
    log "Downloaded ${desc} successfully."
}

if [ "${BIRDNET_SKIP_MODEL_DOWNLOAD}" != "1" ]; then
    mkdir -p "${MODEL_DIR}"

    download_if_missing \
        "${BIRDNET_MODEL}" \
        "${ZENODO_BASE}/${MODEL_FILE}/content" \
        "BirdNET+ V3.0 model (~541 MB FP32 ONNX)"

    download_if_missing \
        "${BIRDNET_LABELS}" \
        "${ZENODO_BASE}/${LABELS_FILE}/content" \
        "species labels CSV"
else
    log "BIRDNET_SKIP_MODEL_DOWNLOAD=1 — skipping model download."
    [ -f "${BIRDNET_MODEL}" ]  || warn "Model not found at ${BIRDNET_MODEL}"
    [ -f "${BIRDNET_LABELS}" ] || warn "Labels not found at ${BIRDNET_LABELS}"
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
if [ -z "${BIRDNET_ALSA_DEVICE}" ] \
    && [ -z "${BIRDNET_PIPEWIRE_DEVICE}" ] \
    && [ -z "${BIRDNET_RTSP_URL}" ] \
    && [ -z "${BIRDNET_RTSP_URLS}" ]; then
    warn "No audio source configured."
    warn "Set one of: BIRDNET_ALSA_DEVICE, BIRDNET_PIPEWIRE_DEVICE,"
    warn "            BIRDNET_RTSP_URL, or BIRDNET_RTSP_URLS."
    warn "The web UI will start but no detections will be made."
    warn "In file-watch mode, drop WAV files into \$BIRDNET_WATCH_DIR instead."
fi

# ---------------------------------------------------------------------------
# Launch
# ---------------------------------------------------------------------------
log "Starting birdnet-behavior (listen: ${BIRDNET_LISTEN})"
exec /usr/local/bin/birdnet-behavior "$@"
