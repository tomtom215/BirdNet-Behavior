#!/usr/bin/env bash
# install.sh — BirdNet-Behavior installer for Raspberry Pi and x86_64 Linux
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/tomtom215/BirdNet-Behavior/main/install.sh | bash
#   # or, for a specific version:
#   VERSION=0.2.0 bash install.sh
#
# What this script does:
#   1. Detects the system architecture (aarch64 / x86_64)
#   2. Downloads the pre-built binary from GitHub Releases
#   3. Creates configuration, data, and recording directories
#   4. Installs a systemd service unit (birdnet-behavior.service)
#   5. Optionally prompts for ALSA device / RTSP URL
#
# Requirements: curl or wget, systemd

set -euo pipefail

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

REPO="tomtom215/BirdNet-Behavior"
BINARY_NAME="birdnet-behavior"
INSTALL_DIR="/usr/local/bin"
CONFIG_DIR="/etc/birdnet"
CONFIG_FILE="${CONFIG_DIR}/birdnet.conf"
DATA_DIR="${HOME}/BirdNet-Behavior"
RECS_DIR="${DATA_DIR}/recordings"
IMAGE_CACHE_DIR="${DATA_DIR}/image_cache"
MODEL_DIR="${DATA_DIR}/models"
DB_PATH="${DATA_DIR}/birds.db"
SERVICE_FILE="/etc/systemd/system/birdnet-behavior.service"
SERVICE_USER="${SUDO_USER:-${USER}}"
LISTEN_ADDR="0.0.0.0:8502"

# BirdNET+ V3.0 model files (Zenodo — direct download, no login required).
# FP32 ONNX (~541 MB): same model used by BirdNET-Pi, works on all platforms.
ZENODO_RECORD="18247420"
MODEL_FILE="BirdNET+_V3.0-preview3_Global_11K_FP32.onnx"
LABELS_FILE="BirdNET+_V3.0-preview3_Global_11K_Labels.csv"
# Use the Zenodo API content endpoint (handles + in filenames correctly).
ZENODO_API="https://zenodo.org/api/records/${ZENODO_RECORD}/files"

# Colour codes (used only when stdout is a terminal)
if [ -t 1 ]; then
    RED='\033[0;31m'
    GREEN='\033[0;32m'
    YELLOW='\033[1;33m'
    BLUE='\033[0;34m'
    BOLD='\033[1m'
    RESET='\033[0m'
else
    RED='' GREEN='' YELLOW='' BLUE='' BOLD='' RESET=''
fi

info()    { echo -e "${BLUE}[INFO]${RESET}  $*"; }
success() { echo -e "${GREEN}[OK]${RESET}    $*"; }
warn()    { echo -e "${YELLOW}[WARN]${RESET}  $*"; }
error()   { echo -e "${RED}[ERROR]${RESET} $*" >&2; }
fatal()   { error "$*"; exit 1; }

# ---------------------------------------------------------------------------
# Root / privilege check
# ---------------------------------------------------------------------------

require_root() {
    if [ "$(id -u)" -ne 0 ]; then
        fatal "This installer must be run as root. Try: curl ... | sudo bash"
    fi
    # Determine who to run the service as.  When invoked via `sudo`, $SUDO_USER
    # is the original (non-root) user.  Refuse to run as root directly so the
    # service doesn't end up owned by root.
    if [ -z "${SUDO_USER:-}" ] || [ "${SUDO_USER}" = "root" ]; then
        fatal "Run the installer with sudo from a normal user account, not as root directly."
    fi
    SERVICE_USER="${SUDO_USER}"
}

# ---------------------------------------------------------------------------
# Architecture detection
# ---------------------------------------------------------------------------

detect_arch() {
    local machine
    machine="$(uname -m)"
    case "${machine}" in
        aarch64 | arm64) echo "aarch64-unknown-linux-gnu" ;;
        x86_64)          echo "x86_64-unknown-linux-gnu" ;;
        armv7l)          echo "armv7-unknown-linux-gnueabihf" ;;
        *)
            fatal "Unsupported architecture: ${machine}. Supported: aarch64, x86_64, armv7l."
            ;;
    esac
}

# ---------------------------------------------------------------------------
# Download helper (curl or wget)
# ---------------------------------------------------------------------------

download() {
    local url="$1"
    local dest="$2"
    if command -v curl &>/dev/null; then
        curl -fsSL -L --retry 3 --retry-delay 2 -o "${dest}" "${url}"
    elif command -v wget &>/dev/null; then
        wget -q --tries=3 -O "${dest}" "${url}"
    else
        fatal "Neither curl nor wget is available. Please install one and retry."
    fi
}

# ---------------------------------------------------------------------------
# Resolve version to install
# ---------------------------------------------------------------------------

resolve_version() {
    if [ -n "${VERSION:-}" ]; then
        echo "${VERSION}"
        return
    fi
    info "Querying latest release from GitHub…"
    local api_url="https://api.github.com/repos/${REPO}/releases/latest"
    local tmp
    tmp="$(mktemp)"
    if download "${api_url}" "${tmp}" 2>/dev/null; then
        local ver
        ver="$(grep '"tag_name"' "${tmp}" | sed -E 's/.*"v?([^"]+)".*/\1/' | head -1)"
        rm -f "${tmp}"
        if [ -n "${ver}" ]; then
            echo "${ver}"
            return
        fi
    fi
    rm -f "${tmp}"
    fatal "Could not determine latest release version. Set VERSION=x.y.z to install a specific version."
}

# ---------------------------------------------------------------------------
# Download and install binary
# ---------------------------------------------------------------------------

install_binary() {
    local version="$1"
    local arch="$2"

    local artifact="${BINARY_NAME}-${arch}"
    local url="https://github.com/${REPO}/releases/download/v${version}/${artifact}"
    local tmp
    tmp="$(mktemp)"

    info "Downloading ${BINARY_NAME} v${version} for ${arch}…"
    if ! download "${url}" "${tmp}"; then
        rm -f "${tmp}"
        fatal "Download failed. Check that release v${version} exists for ${arch}."
    fi

    install -m 0755 "${tmp}" "${INSTALL_DIR}/${BINARY_NAME}"
    rm -f "${tmp}"
    success "Binary installed to ${INSTALL_DIR}/${BINARY_NAME}"
}

# ---------------------------------------------------------------------------
# Download BirdNET+ V3.0 model from Zenodo
# ---------------------------------------------------------------------------

download_model() {
    local model_dest="${MODEL_DIR}/${MODEL_FILE}"
    local labels_dest="${MODEL_DIR}/${LABELS_FILE}"

    # Skip if already present (re-running installer).
    if [ -f "${model_dest}" ] && [ -f "${labels_dest}" ]; then
        success "Model already downloaded at ${MODEL_DIR} — skipping."
        return
    fi

    info "Downloading BirdNET+ V3.0 model (~541 MB FP32 ONNX) from Zenodo…"
    info "  This may take a few minutes on a slow connection."

    install -d -m 0755 -o "${SERVICE_USER}" -g "${SERVICE_USER}" "${MODEL_DIR}"

    # Model (Zenodo API /content endpoint handles + in filenames correctly)
    if [ ! -f "${model_dest}" ]; then
        download "${ZENODO_API}/${MODEL_FILE}/content" "${model_dest}" \
            || fatal "Model download failed. Check your internet connection and retry."
        chown "${SERVICE_USER}:${SERVICE_USER}" "${model_dest}"
        success "Model downloaded to ${model_dest}"
    fi

    # Labels
    if [ ! -f "${labels_dest}" ]; then
        download "${ZENODO_API}/${LABELS_FILE}/content" "${labels_dest}" \
            || fatal "Labels download failed."
        chown "${SERVICE_USER}:${SERVICE_USER}" "${labels_dest}"
        success "Labels downloaded to ${labels_dest}"
    fi
}

# ---------------------------------------------------------------------------
# Create directories
# ---------------------------------------------------------------------------

create_directories() {
    info "Creating data directories…"
    # Directories owned by the service user, not root.
    install -d -m 0755 -o "${SERVICE_USER}" -g "${SERVICE_USER}" \
        "${DATA_DIR}" \
        "${RECS_DIR}" \
        "${IMAGE_CACHE_DIR}" \
        "${MODEL_DIR}" \
        "${DATA_DIR}/backups"
    install -d -m 0755 "${CONFIG_DIR}"
    success "Directories created under ${DATA_DIR}"
}

# ---------------------------------------------------------------------------
# Write default config
# ---------------------------------------------------------------------------

write_config() {
    if [ -f "${CONFIG_FILE}" ]; then
        warn "Config file already exists at ${CONFIG_FILE} — skipping."
        return
    fi

    info "Writing default config to ${CONFIG_FILE}…"
    cat > "${CONFIG_FILE}" <<EOF
# BirdNet-Behavior configuration
# Generated by install.sh on $(date -u +"%Y-%m-%d %H:%M UTC")
#
# Edit this file then restart: sudo systemctl restart birdnet-behavior

# --- Paths ---
DB_PATH=${DB_PATH}
RECS_DIR=${RECS_DIR}
IMAGE_CACHE_DIR=${IMAGE_CACHE_DIR}

# --- Model (BirdNET+ V3.0, downloaded automatically by installer) ---
MODEL_PATH=${MODEL_DIR}/${MODEL_FILE}
LABELS_PATH=${MODEL_DIR}/${LABELS_FILE}

# --- Audio source ---
# Use one of: ALSA microphone, RTSP stream, or an existing recordings directory.
# REC_CARD=plughw:1,0
# RTSP_STREAM=rtsp://camera.local:554/stream

# --- Location (used for species frequency filtering and BirdWeather) ---
# LATITUDE=51.5074
# LONGITUDE=-0.1278

# --- Detection ---
# CONFIDENCE=0.7
# SENSITIVITY=1.0
# OVERLAP=0.0
# DATABASE_LANG=en

# --- Disk management ---
# MAX_FILES_SPECIES=100
# DISK_PURGE_THRESHOLD=95

# --- Notifications (Apprise) ---
# APPRISE_URL=http://localhost:8000
# NOTIFY_TRIGGER=each
# WEEKLY_REPORT_SCHEDULE=monday

# --- BirdWeather ---
# BIRDWEATHER_TOKEN=your-token-here

# --- Site name shown in web UI ---
# SITENAME=My Bird Station
EOF
    chmod 0644 "${CONFIG_FILE}"
    success "Default config written — edit ${CONFIG_FILE} to configure your station."
}

# ---------------------------------------------------------------------------
# Install systemd service
# ---------------------------------------------------------------------------

install_service() {
    info "Installing systemd service…"

    cat > "${SERVICE_FILE}" <<EOF
[Unit]
Description=BirdNet-Behavior bird detection and analytics
Documentation=https://github.com/${REPO}
After=network.target sound.target
Wants=network.target

[Service]
Type=simple
User=${SERVICE_USER}
ExecStart=${INSTALL_DIR}/${BINARY_NAME} --config ${CONFIG_FILE} --listen ${LISTEN_ADDR} --watch-dir ${RECS_DIR} --image-cache-dir ${IMAGE_CACHE_DIR}
Restart=on-failure
RestartSec=10
# Allow access to audio devices and files.
SupplementaryGroups=audio
# Limit resource usage.
LimitNOFILE=65536

[Install]
WantedBy=multi-user.target
EOF

    systemctl daemon-reload
    systemctl enable birdnet-behavior.service
    success "Service installed and enabled."
}

# ---------------------------------------------------------------------------
# Detect and configure audio device
# ---------------------------------------------------------------------------

# Returns the first detected ALSA capture device as "plughw:<card>,<device>",
# or an empty string if none found / arecord not available.
detect_first_audio_device() {
    command -v arecord &>/dev/null || return 0
    # arecord -l output looks like: card 1: Device [USB Audio Device], device 0: ...
    local first_card first_device
    first_card="$(arecord -l 2>/dev/null | awk '/^card/{print $2; exit}' | tr -d ':')"
    first_device="$(arecord -l 2>/dev/null | awk '/^card/{match($0,/device ([0-9]+)/,a); print a[1]; exit}')"
    if [ -n "${first_card}" ]; then
        echo "plughw:${first_card},${first_device:-0}"
    fi
}

configure_audio() {
    local device
    device="$(detect_first_audio_device)"

    if [ -n "${device}" ]; then
        info "Auto-detected ALSA device: ${device}"
        # Uncomment and set REC_CARD in the config file.
        sed -i "s|# REC_CARD=plughw:1,0|REC_CARD=${device}|" "${CONFIG_FILE}"
        success "Audio source set to ${device} in ${CONFIG_FILE}"
    else
        warn "No ALSA recording devices found."
        warn "Edit ${CONFIG_FILE} to set REC_CARD or RTSP_STREAM before starting."
    fi
}

# ---------------------------------------------------------------------------
# Start service if audio is configured
# ---------------------------------------------------------------------------

maybe_start_service() {
    # Check whether an audio source was written into the config.
    if grep -qE '^(REC_CARD|RTSP_STREAM)=' "${CONFIG_FILE}" 2>/dev/null; then
        info "Audio source detected in config — starting service now…"
        systemctl start birdnet-behavior.service
        success "Service started."
    else
        warn "No audio source configured yet."
        warn "Edit ${CONFIG_FILE}, then: sudo systemctl start birdnet-behavior"
    fi
}

# ---------------------------------------------------------------------------
# Print post-install instructions
# ---------------------------------------------------------------------------

print_summary() {
    local ip
    ip="$(hostname -I 2>/dev/null | awk '{print $1}' || echo 'localhost')"

    echo
    echo -e "${BOLD}${GREEN}Installation complete!${RESET}"
    echo
    echo -e "  ${BOLD}Binary:${RESET}  ${INSTALL_DIR}/${BINARY_NAME}"
    echo -e "  ${BOLD}Config:${RESET}  ${CONFIG_FILE}"
    echo -e "  ${BOLD}Data:${RESET}    ${DATA_DIR}"
    echo -e "  ${BOLD}Web UI:${RESET}  http://${ip}:8502"
    echo
    if systemctl is-active --quiet birdnet-behavior.service 2>/dev/null; then
        echo -e "${GREEN}Service is running.${RESET} Open http://${ip}:8502 in your browser."
    else
        echo -e "${BOLD}Next steps:${RESET}"
        echo "  1. Set your audio source in ${CONFIG_FILE}:"
        echo "       REC_CARD=plughw:1,0       (ALSA microphone)"
        echo "       RTSP_STREAM=rtsp://…      (RTSP camera)"
        echo
        echo "  2. (Optional) Set LATITUDE and LONGITUDE for species filtering."
        echo
        echo "  3. sudo systemctl start birdnet-behavior"
    fi
    echo
    echo "  Logs:  sudo journalctl -u birdnet-behavior -f"
    echo
}

# ---------------------------------------------------------------------------
# Uninstall helper
# ---------------------------------------------------------------------------

do_uninstall() {
    require_root
    info "Stopping and removing BirdNet-Behavior…"
    systemctl stop birdnet-behavior.service 2>/dev/null || true
    systemctl disable birdnet-behavior.service 2>/dev/null || true
    rm -f "${SERVICE_FILE}"
    systemctl daemon-reload
    rm -f "${INSTALL_DIR}/${BINARY_NAME}"
    success "Binary and service removed."
    warn "Data and config preserved at ${DATA_DIR} and ${CONFIG_FILE}."
    warn "Remove them manually if no longer needed."
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

main() {
    echo -e "${BOLD}BirdNet-Behavior Installer${RESET}"
    echo "  Repository: https://github.com/${REPO}"
    echo

    if [ "${1:-}" = "uninstall" ]; then
        do_uninstall
        exit 0
    fi

    require_root

    local arch version
    arch="$(detect_arch)"
    version="$(resolve_version)"

    info "Arch: ${arch}, Version: ${version}"

    install_binary "${version}" "${arch}"
    create_directories
    download_model
    write_config
    configure_audio
    install_service
    maybe_start_service
    print_summary
}

main "$@"
