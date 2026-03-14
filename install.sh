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
DB_PATH="${DATA_DIR}/birds.db"
SERVICE_FILE="/etc/systemd/system/birdnet-behavior.service"
SERVICE_USER="${SUDO_USER:-${USER}}"
LISTEN_ADDR="0.0.0.0:8502"

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
        fatal "This installer must be run as root (use sudo)."
    fi
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
        curl -fsSL --retry 3 --retry-delay 2 -o "${dest}" "${url}"
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
# Create directories
# ---------------------------------------------------------------------------

create_directories() {
    info "Creating data directories…"
    # Directories owned by the service user, not root.
    install -d -m 0755 -o "${SERVICE_USER}" -g "${SERVICE_USER}" \
        "${DATA_DIR}" \
        "${RECS_DIR}" \
        "${IMAGE_CACHE_DIR}" \
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

# --- Model (download from https://github.com/kahst/BirdNET-Analyzer) ---
# MODEL=/home/${SERVICE_USER}/BirdNET-Analyzer/checkpoints/V2.4/BirdNET_GLOBAL_6K_V2.4_Model_FP32.tflite
# LABELS=/home/${SERVICE_USER}/BirdNET-Analyzer/checkpoints/V2.4/BirdNET_GLOBAL_6K_V2.4_Labels.txt

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
    chmod 0640 "${CONFIG_FILE}"
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
    success "Service installed and enabled (but not started — edit config first)."
}

# ---------------------------------------------------------------------------
# Check for audio devices (informational)
# ---------------------------------------------------------------------------

check_audio_devices() {
    if command -v arecord &>/dev/null; then
        local cards
        cards="$(arecord -l 2>/dev/null | grep '^card' || true)"
        if [ -n "${cards}" ]; then
            info "Detected ALSA recording devices:"
            echo "${cards}" | while IFS= read -r line; do
                echo "    ${line}"
            done
            info "Set REC_CARD in ${CONFIG_FILE} (format: plughw:<card>,<device>)."
        else
            warn "No ALSA recording devices found. Use an RTSP stream instead."
        fi
    fi
}

# ---------------------------------------------------------------------------
# Print post-install instructions
# ---------------------------------------------------------------------------

print_summary() {
    echo
    echo -e "${BOLD}${GREEN}Installation complete!${RESET}"
    echo
    echo -e "  ${BOLD}Binary:${RESET}  ${INSTALL_DIR}/${BINARY_NAME}"
    echo -e "  ${BOLD}Config:${RESET}  ${CONFIG_FILE}"
    echo -e "  ${BOLD}Data:${RESET}    ${DATA_DIR}"
    echo -e "  ${BOLD}Web UI:${RESET}  http://$(hostname -I 2>/dev/null | awk '{print $1}' || echo 'localhost'):8502"
    echo
    echo -e "${BOLD}Next steps:${RESET}"
    echo "  1. Download the BirdNET model and labels:"
    echo "       https://github.com/kahst/BirdNET-Analyzer/releases"
    echo "     Then update MODEL= and LABELS= in ${CONFIG_FILE}"
    echo
    echo "  2. Configure your audio source in ${CONFIG_FILE}:"
    echo "       REC_CARD=plughw:1,0   (ALSA microphone)"
    echo "       RTSP_STREAM=rtsp://…  (RTSP camera)"
    echo
    echo "  3. (Optional) Set LATITUDE and LONGITUDE for species frequency filtering."
    echo
    echo "  4. Start the service:"
    echo "       sudo systemctl start birdnet-behavior"
    echo "       sudo systemctl status birdnet-behavior"
    echo
    echo "  5. Open the web UI at http://$(hostname -I 2>/dev/null | awk '{print $1}' || echo 'localhost'):8502"
    echo
    echo "  To view logs:"
    echo "       sudo journalctl -u birdnet-behavior -f"
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
    write_config
    install_service
    check_audio_devices
    print_summary
}

main "$@"
