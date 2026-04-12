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
STREAM_DIR="/tmp/birdnet-stream"
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
        armv6l | armv7l)
            # The `ort` crate does not ship prebuilt ONNX Runtime binaries
            # for armv7, so BirdNet-Behavior does not publish a 32-bit ARM
            # release binary.  Pi 3 / Pi Zero 2W users should install the
            # 64-bit Raspberry Pi OS and use the aarch64 binary, or build
            # from source.  See RELEASING.md.
            fatal "Unsupported architecture: ${machine}. 32-bit ARM is not supported; install the 64-bit Raspberry Pi OS and re-run this script, or build from source."
            ;;
        *)
            fatal "Unsupported architecture: ${machine}. Supported: aarch64, x86_64."
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
#
# Release artifacts are gzipped tarballs of the form
#   birdnet-behavior-<version>-<target>.tar.gz
# containing a single top-level directory with the stripped binary alongside
# README, LICENSE, LICENSE-UPSTREAM, CHANGELOG, and this script. A single
# SHA256SUMS file is attached to each GitHub Release for verification.
# ---------------------------------------------------------------------------

install_binary() {
    local version="$1"
    local arch="$2"

    local archive="${BINARY_NAME}-${version}-${arch}.tar.gz"
    local base_url="https://github.com/${REPO}/releases/download/v${version}"
    local archive_url="${base_url}/${archive}"
    local sums_url="${base_url}/SHA256SUMS"

    local workdir
    workdir="$(mktemp -d)"
    # shellcheck disable=SC2064
    trap "rm -rf '${workdir}'" RETURN

    info "Downloading ${archive}…"
    if ! download "${archive_url}" "${workdir}/${archive}"; then
        fatal "Archive download failed. Check that release v${version} exists for ${arch}."
    fi

    info "Downloading SHA256SUMS for verification…"
    if download "${sums_url}" "${workdir}/SHA256SUMS" 2>/dev/null; then
        # sha256sum -c expects files referenced in SHA256SUMS to be present
        # in the working directory, so verify from inside workdir.
        if (cd "${workdir}" && sha256sum -c SHA256SUMS --ignore-missing --status --strict) 2>/dev/null; then
            success "Checksum verified against SHA256SUMS"
        else
            fatal "Checksum mismatch for ${archive} against published SHA256SUMS. Aborting install."
        fi
    else
        warn "SHA256SUMS could not be downloaded — continuing without checksum verification."
    fi

    info "Extracting archive…"
    if ! tar -xzf "${workdir}/${archive}" -C "${workdir}"; then
        fatal "Archive extraction failed. The downloaded file may be corrupt."
    fi

    # The archive contains a single top-level directory named
    # birdnet-behavior-<version>-<target>. Locate the binary inside it.
    local extracted_binary
    extracted_binary="$(find "${workdir}" -mindepth 2 -maxdepth 3 -type f -name "${BINARY_NAME}" | head -1)"
    if [ -z "${extracted_binary}" ] || [ ! -f "${extracted_binary}" ]; then
        fatal "Could not find '${BINARY_NAME}' binary inside the downloaded archive."
    fi

    install -m 0755 "${extracted_binary}" "${INSTALL_DIR}/${BINARY_NAME}"
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

setup_tmpfs_streaming() {
    info "Setting up tmpfs for audio streaming (SD card wear protection)…"
    # Use /tmp/birdnet-stream for raw audio capture. On most Pi distros /tmp is
    # already a tmpfs; this ensures the streaming directory exists after reboot.
    install -d -m 0755 -o "${SERVICE_USER}" -g "${SERVICE_USER}" "${STREAM_DIR}"

    # If /tmp is NOT already tmpfs, create a dedicated mount.
    if ! findmnt -t tmpfs /tmp &>/dev/null; then
        local MOUNT_UNIT="/etc/systemd/system/tmp-birdnet\\x2dstream.mount"
        cat > "${MOUNT_UNIT}" <<MEOF
[Unit]
Description=tmpfs for BirdNet-Behavior audio streaming
Before=birdnet-behavior.service

[Mount]
What=tmpfs
Where=${STREAM_DIR}
Type=tmpfs
Options=size=64M,mode=0755,uid=$(id -u "${SERVICE_USER}"),gid=$(id -g "${SERVICE_USER}")

[Install]
WantedBy=multi-user.target
MEOF
        systemctl daemon-reload
        systemctl enable --now "tmp-birdnet\\x2dstream.mount" 2>/dev/null || true
        success "tmpfs mount unit installed for ${STREAM_DIR}"
    else
        success "/tmp is already tmpfs — ${STREAM_DIR} is RAM-backed"
    fi
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
StartLimitBurst=5
StartLimitIntervalSec=300

[Service]
Type=simple
User=${SERVICE_USER}
ExecStart=${INSTALL_DIR}/${BINARY_NAME} --config ${CONFIG_FILE} --listen ${LISTEN_ADDR} --watch-dir ${STREAM_DIR} --image-cache-dir ${IMAGE_CACHE_DIR}
# Always restart — covers panics (SIGABRT with panic=abort), OOM kills, and errors.
Restart=always
RestartSec=10
# systemd watchdog: process must notify within this interval or gets killed + restarted.
WatchdogSec=120
# Hard memory ceiling — prevents OOM-killing other processes on low-RAM Pis.
MemoryMax=512M
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
# ZRAM compressed swap (optional — Pi Zero 2W and low-RAM boards)
# ---------------------------------------------------------------------------

# Install and enable a ZRAM swap device sized at half of physical RAM.
#
# ZRAM uses in-RAM compression rather than swapping to SD card, which:
#   - Dramatically reduces SD card wear (no swap writes to disk)
#   - Provides more effective working memory on Pi Zero 2W (512 MB RAM)
#   - Is transparent to the OS and BirdNet-Behavior
#
# Requires kernel >= 3.15 (all Pi models supported by BirdNET-Pi ship this).
# BirdNET-Pi equivalent: install_zram_service.sh
setup_zram() {
    info "Setting up ZRAM compressed swap…"

    # Check for zramctl (util-linux) — available on Raspberry Pi OS Bullseye+
    if ! command -v zramctl &>/dev/null; then
        warn "zramctl not found — installing util-linux…"
        apt-get install -y util-linux &>/dev/null || {
            warn "Could not install util-linux. Skipping ZRAM setup."
            return 0
        }
    fi

    local mem_bytes
    mem_bytes="$(awk '/MemTotal/ {print $2 * 1024}' /proc/meminfo)"
    local zram_size=$(( mem_bytes / 2 ))   # 50% of physical RAM

    # Load the zram kernel module
    if ! lsmod | grep -q '^zram'; then
        modprobe zram num_devices=1 || {
            warn "Could not load zram module. Skipping ZRAM setup."
            return 0
        }
    fi

    local zram_dev
    zram_dev="$(zramctl --find --size "${zram_size}" --algorithm lz4 2>/dev/null)" || {
        warn "zramctl failed to allocate device. Skipping ZRAM setup."
        return 0
    }

    mkswap "${zram_dev}" &>/dev/null
    swapon --priority 100 "${zram_dev}" || {
        warn "Failed to activate ZRAM swap device. Skipping."
        return 0
    }

    success "ZRAM swap activated: ${zram_dev} ($(( zram_size / 1024 / 1024 )) MB, lz4)"

    # Persist across reboots via a systemd service unit
    local zram_service="/etc/systemd/system/zram-swap.service"
    cat > "${zram_service}" << EOF
[Unit]
Description=ZRAM compressed swap for BirdNet-Behavior
After=multi-user.target

[Service]
Type=oneshot
RemainAfterExit=yes
ExecStart=/bin/sh -c 'modprobe zram num_devices=1 && zramctl --find --size ${zram_size} --algorithm lz4 | xargs -I{} sh -c "mkswap {} && swapon --priority 100 {}"'
ExecStop=/bin/sh -c 'swapoff -a 2>/dev/null; zramctl --list 2>/dev/null | awk "NR>1{print \$1}" | xargs -r rmmod zram 2>/dev/null || true'

[Install]
WantedBy=multi-user.target
EOF

    systemctl daemon-reload
    systemctl enable zram-swap.service &>/dev/null
    success "ZRAM swap service installed and enabled (persists across reboots)."
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
    setup_tmpfs_streaming
    download_model
    write_config
    configure_audio
    install_service

    # Offer ZRAM compressed swap on boards with ≤ 2 GB RAM (Pi Zero 2W, Pi 2, etc.)
    # Silently skipped on machines with adequate RAM or where ZRAM is unavailable.
    local mem_mb
    mem_mb="$(awk '/MemTotal/ {printf "%d", $2/1024}' /proc/meminfo 2>/dev/null || echo 9999)"
    if [ "${mem_mb}" -le 2048 ] && [ "${SKIP_ZRAM:-0}" != "1" ]; then
        info "Low-RAM system detected (${mem_mb} MB) — setting up ZRAM compressed swap…"
        setup_zram || warn "ZRAM setup failed (non-fatal); continuing without it."
    fi

    maybe_start_service
    print_summary
}

main "$@"
