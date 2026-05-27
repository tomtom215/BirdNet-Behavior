#!/usr/bin/env bash
# install.sh — BirdNet-Behavior installer for Raspberry Pi and x86_64 Linux
#
# Usage (Linux / Raspberry Pi — installs a systemd service, so it needs root):
#   curl -fsSL https://raw.githubusercontent.com/tomtom215/BirdNet-Behavior/main/install.sh | sudo bash
#   # pin a specific version (the `-s --` passes the args through to the script):
#   curl -fsSL https://raw.githubusercontent.com/tomtom215/BirdNet-Behavior/main/install.sh | sudo bash -s -- --version 0.5.1
#   # from a saved copy:
#   sudo bash install.sh [--version 0.5.1]
#
# Do NOT use `sudo bash <(curl ...)`: process substitution hands bash a file
# descriptor owned by your user, and sudo closes it crossing to root, so the
# script disappears ("/dev/fd/63: No such file or directory"). Use the pipe.
#
# macOS (Apple Silicon) sets up a per-user launchd agent instead — run without sudo.
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
# Default data dir. NOTE: under sudo $HOME is usually /root, so require_root
# re-derives this and the paths below from the service user's real home.
DATA_DIR="${HOME}/BirdNet-Behavior"
RECS_DIR="${DATA_DIR}/recordings"
STREAM_DIR="/tmp/birdnet-stream"
IMAGE_CACHE_DIR="${DATA_DIR}/image_cache"
MODEL_DIR="${DATA_DIR}/models"
DB_PATH="${DATA_DIR}/birds.db"
SERVICE_FILE="/etc/systemd/system/birdnet-behavior.service"
SERVICE_USER="${SUDO_USER:-${USER:-$(id -un)}}"
LISTEN_ADDR="0.0.0.0:8502"

# Interactive onboarding state. INTERACTIVE is decided in main(); the *_VALUE
# vars hold answers from prompt_station_settings and are baked into the config
# by write_config (empty = keep the commented example in place).
INTERACTIVE=0
ALSA_CARD_VALUE=""
RTSP_URL_VALUE=""
LATITUDE_VALUE=""
LONGITUDE_VALUE=""

# Minimum glibc the prebuilt binaries link against (pyke's ONNX Runtime needs
# glibc >= 2.38; Ubuntu 24.04, where we build, ships 2.39). Set
# BIRDNET_SKIP_GLIBC_CHECK=1 to bypass (e.g. you built from source).
REQUIRED_GLIBC="2.39"

# Set to 1 by main() when an already-running service is stopped for an upgrade,
# so we know to restart it afterwards rather than leave it down.
SERVICE_WAS_RUNNING=0

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

# All logging goes to stderr so that stdout is reserved for a function's return
# value. resolve_version/detect_arch return via `echo`, and callers capture them
# with $(...) — a log line on stdout would be swallowed into the value (e.g. a
# version of "[INFO] Querying…\n0.5.1", which then corrupts the download URL).
info()    { echo -e "${BLUE}[INFO]${RESET}  $*" >&2; }
success() { echo -e "${GREEN}[OK]${RESET}    $*" >&2; }
warn()    { echo -e "${YELLOW}[WARN]${RESET}  $*" >&2; }
error()   { echo -e "${RED}[ERROR]${RESET} $*" >&2; }
fatal()   { error "$*"; exit 1; }

# Interactive prompt helpers. They read from /dev/tty (not stdin) so they work
# under the recommended `curl ... | sudo bash`, where stdin is the script text;
# output goes to /dev/tty for the same reason. Gated by INTERACTIVE (set in main).
ask() {
    local prompt="$1" default="${2:-}" reply
    if [ -n "${default}" ]; then
        printf '%s [%s]: ' "${prompt}" "${default}" >/dev/tty
    else
        printf '%s: ' "${prompt}" >/dev/tty
    fi
    read -r reply </dev/tty || reply=""
    printf '%s' "${reply:-${default}}"
}

yesno() {
    local prompt="$1" default="${2:-y}" reply hint="[Y/n]"
    [ "${default}" = "n" ] && hint="[y/N]"
    printf '%s %s ' "${prompt}" "${hint}" >/dev/tty
    read -r reply </dev/tty || reply=""
    case "${reply:-${default}}" in [yY]*) return 0 ;; *) return 1 ;; esac
}

# ---------------------------------------------------------------------------
# Root / privilege check
# ---------------------------------------------------------------------------

require_root() {
    if [ "$(id -u)" -ne 0 ]; then
        error "This installer needs root — it installs a systemd service."
        cat >&2 <<EOF

Re-run it by piping into sudo:

    curl -fsSL https://raw.githubusercontent.com/${REPO}/main/install.sh | sudo bash

To pin a version, pass it as an argument after a literal --:

    curl -fsSL https://raw.githubusercontent.com/${REPO}/main/install.sh | sudo bash -s -- --version ${VERSION:-X.Y.Z}

Already saved the script?   sudo bash install.sh [--version ${VERSION:-X.Y.Z}]

Avoid  sudo bash <(curl ...)  — process substitution gives bash a file
descriptor owned by your user, and sudo closes it on the way to root, so the
script vanishes ("/dev/fd/63: No such file or directory"). The pipe above works.
EOF
        exit 1
    fi
    # Determine who to run the service as.  When invoked via `sudo`, $SUDO_USER
    # is the original (non-root) user.  Refuse to run as root directly so the
    # service doesn't end up owned by root.
    if [ -z "${SUDO_USER:-}" ] || [ "${SUDO_USER}" = "root" ]; then
        fatal "Run the installer via sudo from a normal user account, not as root directly, so the service isn't owned by root.  E.g.:  curl -fsSL https://raw.githubusercontent.com/${REPO}/main/install.sh | sudo bash"
    fi
    SERVICE_USER="${SUDO_USER}"

    # Under sudo, $HOME is usually /root, not the service user's home — so the
    # data dir computed at the top of this script can land in /root, which the
    # non-root service user cannot reach (and ProtectHome=read-only would block
    # it anyway). Re-derive every home-based path from the service user's actual
    # home so the daemon can read its database, recordings, and model.
    local svc_home
    svc_home="$(getent passwd "${SERVICE_USER}" | cut -d: -f6)"
    if [ -n "${svc_home}" ]; then
        DATA_DIR="${svc_home}/BirdNet-Behavior"
        RECS_DIR="${DATA_DIR}/recordings"
        IMAGE_CACHE_DIR="${DATA_DIR}/image_cache"
        MODEL_DIR="${DATA_DIR}/models"
        DB_PATH="${DATA_DIR}/birds.db"
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
        armv6l | armv7l)
            # The `ort` crate does not ship prebuilt ONNX Runtime binaries
            # for armv7 / armv6, so BirdNet-Behavior does not publish a
            # 32-bit ARM release binary.  Every modern Raspberry Pi (Pi 3,
            # Pi 4, Pi 5, Pi 400, Pi Zero 2 W) is 64-bit-capable — if you
            # are seeing this message you are almost certainly running the
            # 32-bit Pi OS on 64-bit-capable hardware.  The fix is to
            # reflash with the 64-bit image, which is the Pi Foundation's
            # current recommendation anyway.
            cat >&2 <<'EOT'

  Unsupported architecture: 32-bit ARM (armv6l / armv7l).

  BirdNet-Behavior does not publish 32-bit ARM release binaries because
  the ONNX Runtime crate (`ort`) ships no prebuilt libraries for that
  target.

  If you are on a Raspberry Pi 3, 4, 5, 400, or Zero 2 W, your hardware
  is 64-bit-capable and you are just running the 32-bit Pi OS.  Reflash
  with the 64-bit image (this is the current Raspberry Pi Foundation
  recommendation) and re-run this installer:

      https://downloads.raspberrypi.com/raspios_arm64/images/

  After reflashing, `uname -m` should print `aarch64`, and this script
  will download the aarch64 release binary automatically.

  Only the original 2015 Pi 2 v1.1 (Cortex-A7) and the Pi 1 / Pi Zero /
  Pi Zero W (ARM11) lack a 64-bit mode entirely.  Those boards would
  need a from-source build, which is not currently supported upstream.

EOT
            fatal "Unsupported architecture: ${machine}."
            ;;
        *)
            fatal "Unsupported architecture: ${machine}. Supported: aarch64, x86_64."
            ;;
    esac
}

# ---------------------------------------------------------------------------
# glibc preflight
#
# The prebuilt release binaries are built on Ubuntu 24.04 (glibc 2.39) because
# pyke's prebuilt ONNX Runtime requires glibc >= 2.38. On an older system
# (notably Raspberry Pi OS Bookworm / Debian 12, glibc 2.36) the binary loads
# but dies with "version `GLIBC_2.39' not found". Catch that here, before we
# download 540 MB of model, and point the user at a path that actually works.
# ---------------------------------------------------------------------------

detect_glibc_version() {
    # Prefer getconf (prints "glibc 2.36"); fall back to `ldd --version`.
    local v
    v="$(getconf GNU_LIBC_VERSION 2>/dev/null | awk '{print $2}')"
    if [ -z "${v}" ]; then
        v="$(ldd --version 2>/dev/null | head -1 | grep -oE '[0-9]+\.[0-9]+' | tail -1)"
    fi
    echo "${v}"
}

check_glibc() {
    if [ "${BIRDNET_SKIP_GLIBC_CHECK:-0}" = "1" ]; then
        warn "Skipping glibc check (BIRDNET_SKIP_GLIBC_CHECK=1)."
        return 0
    fi

    local current
    current="$(detect_glibc_version)"
    if [ -z "${current}" ]; then
        warn "Could not determine the system glibc version."
        warn "The prebuilt binary needs glibc >= ${REQUIRED_GLIBC}; continuing anyway."
        return 0
    fi

    # current >= required  iff  the lower of the two (sort -V) is the requirement.
    if [ "$(printf '%s\n%s\n' "${REQUIRED_GLIBC}" "${current}" | sort -V | head -1)" = "${REQUIRED_GLIBC}" ]; then
        success "glibc ${current} (>= ${REQUIRED_GLIBC}) — OK"
        return 0
    fi

    error "System glibc is ${current}, but the prebuilt binary requires >= ${REQUIRED_GLIBC}."
    cat >&2 <<EOT

  Your OS is too old for the prebuilt native binary. Raspberry Pi OS
  Bookworm / Debian 12 ship glibc 2.36 and are NOT supported by the
  binary. Two paths that DO work:

    1. Run the Docker image — it bundles its own runtime, so the host
       glibc does not matter (works fine on Bookworm):

           bash <(curl -fsSL https://raw.githubusercontent.com/${REPO}/main/quickstart.sh)

    2. Upgrade the OS to Raspberry Pi OS Trixie / Debian 13 /
       Ubuntu 24.04 (or newer), then re-run this installer.

  Building from source against your system's older toolchain also works;
  see the documentation. To bypass this check anyway (you know what you
  are doing), re-run with BIRDNET_SKIP_GLIBC_CHECK=1.

EOT
    fatal "Unsupported glibc ${current} (need >= ${REQUIRED_GLIBC})."
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
# Large-file download helper — resumes on interrupt, shows a progress bar
# so the operator sees something is happening during the ~541 MB model pull.
#
#   download_large URL DEST [HUMAN_NAME]
#
# Behaviour:
#   - If DEST already exists, resume from its current byte offset (-C -).
#   - Print a progress bar to the terminal (-#).
#   - Up to 5 automatic retries with exponential backoff for transient errors.
#   - Treat HTTP errors as failures (-f).
#   - Leave the partial file in place on failure so the next run can resume.
# ---------------------------------------------------------------------------
download_large() {
    local url="$1"
    local dest="$2"
    local name="${3:-${dest##*/}}"
    info "  Fetching ${name}…"
    if command -v curl &>/dev/null; then
        # -C - : resume; -# : progress bar; --retry-all-errors handles flaky CDNs.
        curl -fL -C - -# \
            --retry 5 --retry-delay 2 --retry-all-errors --retry-max-time 600 \
            --connect-timeout 30 \
            -o "${dest}" "${url}"
    elif command -v wget &>/dev/null; then
        # -c : resume; --show-progress to stderr; tolerate transient failures.
        wget -c --tries=5 --waitretry=2 --timeout=30 --show-progress -O "${dest}" "${url}"
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
    fatal "Could not determine latest release version. Pass --version x.y.z (or set VERSION=x.y.z) to install a specific version."
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

    if [ -x "${INSTALL_DIR}/${BINARY_NAME}" ]; then
        local old_ver
        old_ver="$("${INSTALL_DIR}/${BINARY_NAME}" --version 2>/dev/null | awk '{print $NF}' || true)"
        if [ -n "${old_ver}" ]; then
            info "Existing install detected (v${old_ver}) — upgrading to v${version}."
        fi
    fi

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

    # Model (Zenodo API /content endpoint handles + in filenames correctly).
    # Uses download_large so a dropped connection picks up where it left off
    # on the next run instead of restarting from 0 MB.
    if [ ! -f "${model_dest}" ]; then
        if ! download_large "${ZENODO_API}/${MODEL_FILE}/content" "${model_dest}" "BirdNET+ V3.0 model (~541 MB)"; then
            warn "Model download was interrupted; the partial file is kept at:"
            warn "  ${model_dest}"
            warn "Re-run this installer to resume from where it stopped."
            warn "Common causes: no internet connection, Zenodo temporarily down, or disk full."
            fatal "Model download failed. Check the cause above and retry."
        fi
        chown "${SERVICE_USER}:${SERVICE_USER}" "${model_dest}"
        success "Model downloaded to ${model_dest}"
    fi

    # Labels (small file — no resume needed, but keep the consistent helper).
    if [ ! -f "${labels_dest}" ]; then
        if ! download "${ZENODO_API}/${LABELS_FILE}/content" "${labels_dest}"; then
            fatal "Labels download failed. Check your internet connection and retry."
        fi
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

    # Bake interactive / auto-detected answers in; otherwise keep the commented
    # examples. Heredoc expansion is single-pass, so a value containing $ or
    # backticks (e.g. an RTSP URL with credentials) is written literally.
    local alsa_line="# ALSA_CARD=plughw:1,0"
    local rtsp_line="# RTSP_URL=rtsp://camera.local:554/stream"
    local lat_line="# LATITUDE=51.5074"
    local lon_line="# LONGITUDE=-0.1278"
    [ -n "${ALSA_CARD_VALUE}" ] && alsa_line="ALSA_CARD=${ALSA_CARD_VALUE}"
    [ -n "${RTSP_URL_VALUE}" ]  && rtsp_line="RTSP_URL=${RTSP_URL_VALUE}"
    [ -n "${LATITUDE_VALUE}" ]  && lat_line="LATITUDE=${LATITUDE_VALUE}"
    [ -n "${LONGITUDE_VALUE}" ] && lon_line="LONGITUDE=${LONGITUDE_VALUE}"

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
${alsa_line}
${rtsp_line}

# --- Location (used for species frequency filtering and BirdWeather) ---
${lat_line}
${lon_line}

# --- Detection ---
# CONFIDENCE=0.25          # 0.0–1.0, default 0.25
# SENSITIVITY=1.0          # 0.5–1.5, default 1.0
# OVERLAP=0.0              # seconds of 3 s analysis window overlap
# SF_THRESH=0.03           # species-frequency metadata-filter threshold
# DATABASE_LANG=en

# --- Disk management ---
# MAX_FILES_SPECIES=100
# DISK_PURGE_THRESHOLD=95

# --- Notifications (Apprise) ---
# APPRISE_URL=http://localhost:8000

# --- BirdWeather ---
# BIRDWEATHER_TOKEN=your-token-here

# --- Site name shown in web UI ---
# SITENAME=My Bird Station

# --- Web UI authentication (recommended) ---
# The web UI — including the admin panel that can change settings, trigger
# database backups, and update the software — listens on ${LISTEN_ADDR} and is
# reachable by anyone on your network. Set a password to require HTTP Basic
# auth; without CADDY_PWD the UI is open to the whole LAN. Username defaults to
# "birdnet". (Alternatively, set LISTEN to 127.0.0.1:8502 and use an SSH tunnel.)
# CADDY_USER=birdnet
# CADDY_PWD=change-me-to-a-strong-password
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
# Wait for the network stack AND sound subsystem before launching. The
# detection daemon needs both; running before them just causes an
# avoidable restart loop on slow-booting hardware (USB enumeration on Pi).
After=network-online.target sound.target time-sync.target
Wants=network-online.target
# Don't enter a tight restart loop. If 5 restarts happen inside 5 min the
# unit is marked failed and stays down for operator review (visible in
# the web UI's health page once the service comes back).
StartLimitBurst=5
StartLimitIntervalSec=300

[Service]
# Type=notify pairs with sd_notify in src/sd_notify.rs:
#   - READY=1 when the web server has bound its socket
#   - WATCHDOG=1 periodic pings keep the watchdog happy
#   - STOPPING=1 on graceful shutdown
Type=notify
NotifyAccess=main
User=${SERVICE_USER}

# Preflight: run the doctor before starting the main service so a broken
# install fails fast with an actionable report in the journal, rather than
# entering a restart loop that fills the disk with logs.
# Exit 0 (pass) or 1 (warnings only) are both accepted — only exit 2
# (errors that will prevent operation) keeps the service from starting.
ExecStartPre=/bin/sh -c '${INSTALL_DIR}/${BINARY_NAME} --doctor --config ${CONFIG_FILE} || [ \$? -le 1 ]'
# DuckDB behavioral analytics is compiled into every release binary and enabled
# here by default (the database is created on first run). To run without it
# (e.g. on a very low-RAM board), remove the --analytics-db flag below.
ExecStart=${INSTALL_DIR}/${BINARY_NAME} --config ${CONFIG_FILE} --listen ${LISTEN_ADDR} --watch-dir ${STREAM_DIR} --image-cache-dir ${IMAGE_CACHE_DIR} --analytics-db ${DATA_DIR}/analytics.db

# Restart policy. panic=abort means panics show up as SIGABRT exits;
# Restart=always covers panics, OOM kills, and any non-zero exit.
Restart=always
RestartSec=10
# Generous startup budget so a first-run model download / DB migration
# doesn't trip the watchdog while it is still legitimately working.
TimeoutStartSec=900
# Allow graceful shutdown to drain WAL and finish outstanding HTTP
# requests; SIGTERM is the friendly signal, SIGKILL is the fallback.
TimeoutStopSec=30
KillSignal=SIGTERM
KillMode=mixed
SendSIGKILL=yes

# Watchdog: src/sd_notify.rs pings every WatchdogSec/2.
# 120 s window is plenty: a healthy daemon pings every ~60 s.
WatchdogSec=120

# Resource ceilings. Tuned conservatively so a runaway process can't
# take down the whole Pi.
MemoryMax=512M
MemoryHigh=384M
TasksMax=512
LimitNOFILE=65536
LimitNPROC=256
# Recover gracefully under memory pressure.
OOMScoreAdjust=200
OOMPolicy=stop

# ── Filesystem isolation ─────────────────────────────────────────────────
# Read-only access to the rest of the filesystem; explicit write paths.
ProtectSystem=strict
ProtectHome=read-only
ReadWritePaths=${DATA_DIR} ${CONFIG_DIR} ${STREAM_DIR} /run /var/log
PrivateTmp=yes
ProtectKernelLogs=yes
ProtectKernelModules=yes
ProtectKernelTunables=yes
ProtectControlGroups=yes
ProtectClock=yes
ProtectHostname=yes
ProtectProc=invisible
ProcSubset=pid
# Block-listed kernel surfaces; we don't need them.
RestrictSUIDSGID=yes
RestrictRealtime=yes
RestrictNamespaces=yes
LockPersonality=yes
MemoryDenyWriteExecute=yes
NoNewPrivileges=yes
SystemCallArchitectures=native
# Permit only POSIX, file I/O, networking, and signals — explicitly
# excludes things like raw_io / module_load / ptrace / mount / reboot.
SystemCallFilter=@system-service
SystemCallFilter=~@privileged @resources @mount @debug @cpu-emulation @obsolete @reboot @swap @raw-io @clock @module

# Audio access — must keep these capability sets / device mounts.
SupplementaryGroups=audio
DeviceAllow=/dev/snd rw
DevicePolicy=closed

# ── Logging ──────────────────────────────────────────────────────────────
StandardOutput=journal
StandardError=journal
SyslogIdentifier=birdnet-behavior
# Cap journal volume so a chatty failure mode can't exhaust the disk on
# a Pi with a small SD card.
LogRateLimitIntervalSec=30
LogRateLimitBurst=1000

[Install]
WantedBy=multi-user.target
EOF

    systemctl daemon-reload
    systemctl enable birdnet-behavior.service
    success "Service installed and enabled (Type=notify, hardened, watchdog active)."
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
    # 2-arg match() (RSTART/RLENGTH) is POSIX; the 3-arg capture form is a gawk
    # extension that errors on mawk (the default awk on Debian / Raspberry Pi OS).
    first_device="$(arecord -l 2>/dev/null | awk '/^card/{ if (match($0, /device [0-9]+/)) print substr($0, RSTART + 7, RLENGTH - 7); exit }')"
    if [ -n "${first_card}" ]; then
        echo "plughw:${first_card},${first_device:-0}"
    fi
}

# True (0) if v is a decimal number within [lo, hi]. Used to sanity-check
# latitude/longitude so a typo doesn't get written into the config.
valid_coord() {
    awk -v v="$1" -v lo="$2" -v hi="$3" \
        'BEGIN { if (v ~ /^[-+]?([0-9]+\.?[0-9]*|\.[0-9]+)$/ && v+0 >= lo && v+0 <= hi) exit 0; exit 1 }'
}

# Collect the audio source and station location on a fresh install. When
# interactive we ask the operator directly (so a non-technical user gets a
# working station without hand-editing a file); otherwise we keep the historical
# behaviour — silently auto-detect ALSA and leave location for the config file.
# The config file always remains editable and is the source of truth afterwards.
prompt_station_settings() {
    # Re-install / upgrade: the config already exists and is the user's own —
    # never re-prompt or overwrite their settings.
    [ -f "${CONFIG_FILE}" ] && return 0

    local candidate
    candidate="$(detect_first_audio_device)"

    if [ "${INTERACTIVE}" != "1" ]; then
        if [ -n "${candidate}" ]; then
            ALSA_CARD_VALUE="${candidate}"
            info "Auto-detected ALSA device: ${candidate}"
        else
            warn "No ALSA device detected — set ALSA_CARD or RTSP_URL in ${CONFIG_FILE}."
        fi
        return 0
    fi

    # ---- Audio source ----
    printf '\n  Audio source\n' >/dev/tty
    if [ -n "${candidate}" ]; then
        arecord -l 2>/dev/null | grep '^card' | sed 's/^/    /' >/dev/tty || true
        yesno "  Use detected ALSA device '${candidate}'?" y && ALSA_CARD_VALUE="${candidate}"
    else
        printf '  No ALSA capture device detected.\n' >/dev/tty
    fi
    if [ -z "${ALSA_CARD_VALUE}" ]; then
        local audio_in
        audio_in="$(ask "  Audio source — ALSA device (e.g. plughw:1,0) or rtsp:// URL (Enter to skip)" "")"
        case "${audio_in}" in
            '')                   : ;;
            rtsp://* | rtsps://*) RTSP_URL_VALUE="${audio_in}" ;;
            *)                    ALSA_CARD_VALUE="${audio_in}" ;;
        esac
    fi
    if [ -n "${ALSA_CARD_VALUE}" ]; then
        success "Audio source: ALSA ${ALSA_CARD_VALUE}"
    elif [ -n "${RTSP_URL_VALUE}" ]; then
        success "Audio source: RTSP ${RTSP_URL_VALUE}"
    else
        warn "No audio source set — add ALSA_CARD or RTSP_URL to ${CONFIG_FILE} later."
    fi

    # ---- Station location ----
    printf '\n  Station location (solar schedule, species filter, BirdWeather)\n' >/dev/tty
    printf '  Tip: right-click your spot on https://openstreetmap.org and read off the coordinates.\n' >/dev/tty
    local lat lon
    lat="$(ask "  Latitude  (e.g. 42.3601, Enter to skip)" "")"
    if [ -n "${lat}" ]; then
        lon="$(ask "  Longitude (e.g. -71.0589)" "")"
        if valid_coord "${lat}" -90 90 && valid_coord "${lon}" -180 180; then
            LATITUDE_VALUE="${lat}"
            LONGITUDE_VALUE="${lon}"
            success "Location: ${lat}, ${lon}"
        else
            warn "Coordinates '${lat}, ${lon}' look invalid — skipping; set LATITUDE/LONGITUDE in ${CONFIG_FILE} later."
        fi
    fi
}

# ---------------------------------------------------------------------------
# Start service if audio is configured
# ---------------------------------------------------------------------------

maybe_start_service() {
    # Upgrade path: if we stopped a running service to swap the binary, bring
    # it back on the new version. Schema migrations run automatically on
    # startup, and the SQLite/DuckDB data + config were left untouched.
    if [ "${SERVICE_WAS_RUNNING}" = "1" ]; then
        info "Restarting service on the upgraded binary…"
        systemctl start birdnet-behavior.service
        success "Service restarted (schema migrations applied on startup)."
        return
    fi

    # Fresh install: only start if an audio source was written into the config.
    if grep -qE '^(ALSA_CARD|RTSP_URL)=' "${CONFIG_FILE}" 2>/dev/null; then
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
        echo "  1. Set an audio source (edit as root):  sudo nano ${CONFIG_FILE}"
        echo "       ALSA_CARD=plughw:1,0      (ALSA microphone)"
        echo "       RTSP_URL=rtsp://…         (RTSP camera)"
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
# macOS (Apple Silicon) install path
#
# The flow above is systemd-specific and would break partway on macOS. macOS
# instead gets a per-user launchd LaunchAgent (no sudo), the aarch64-apple-darwin
# prebuilt when a release publishes one, and clear from-source guidance until
# then — so a Mac user who runs this script never ends up half-installed.
# ---------------------------------------------------------------------------
MAC_DATA_DIR="${HOME}/Library/Application Support/birdnet-behavior"
MAC_PLIST="${HOME}/Library/LaunchAgents/com.tomtom215.birdnet-behavior.plist"

mac_brew_dep() { # $1=formula  $2=why
    command -v "$1" &>/dev/null && { success "${1} present"; return 0; }
    warn "${1} not found — ${2}"
    if ! command -v brew &>/dev/null; then
        warn "Homebrew not found. Install it from https://brew.sh then: brew install ${1}"
        return 1
    fi
    local ans=""
    if [ -t 0 ]; then read -rp "  Install ${1} with Homebrew now? [Y/n] " ans </dev/tty 2>/dev/null || ans=""; fi
    case "$ans" in
        n|N|no|NO) warn "Skipped — install later with: brew install ${1}" ;;
        *) info "Running: brew install ${1}"; brew install "$1" || warn "brew install ${1} failed; continuing" ;;
    esac
}

macos_setup_config_and_agent() { # $1=binary path
    local bin="$1" secret
    mkdir -p "${MAC_DATA_DIR}" "${HOME}/Library/Logs" "$(dirname "${MAC_PLIST}")"
    if [ ! -f "${MAC_DATA_DIR}/birdnet.conf" ]; then
        cat > "${MAC_DATA_DIR}/birdnet.conf" <<CONF
# BirdNet-Behavior config (macOS). Edit LATITUDE/LONGITUDE and set a mic device.
SITENAME=My Backyard
LATITUDE=0.0
LONGITUDE=0.0
DB_PATH=${MAC_DATA_DIR}/birds.db
RECS_DIR=${MAC_DATA_DIR}/recordings
IMAGE_CACHE_DIR=${MAC_DATA_DIR}/image_cache
CONF
        success "Wrote starter config: ${MAC_DATA_DIR}/birdnet.conf"
    else
        info "Keeping existing config: ${MAC_DATA_DIR}/birdnet.conf"
    fi
    secret="$(openssl rand -base64 48 2>/dev/null | tr -d '\n' || echo 'CHANGE-ME-to-32-plus-random-bytes')"
    cat > "${MAC_PLIST}" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>com.tomtom215.birdnet-behavior</string>
  <key>ProgramArguments</key>
  <array>
    <string>${bin}</string>
    <string>-c</string><string>${MAC_DATA_DIR}/birdnet.conf</string>
    <string>--analytics-db</string><string>${MAC_DATA_DIR}/analytics.duckdb</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>${HOME}/Library/Logs/birdnet-behavior.log</string>
  <key>StandardErrorPath</key><string>${HOME}/Library/Logs/birdnet-behavior.err.log</string>
  <key>EnvironmentVariables</key>
  <dict>
    <key>BNB_STATION_LAT</key><string>0.0</string>
    <key>BNB_STATION_LON</key><string>0.0</string>
    <key>BNB_SHARE_SECRET</key><string>${secret}</string>
  </dict>
</dict>
</plist>
PLIST
    success "Wrote LaunchAgent: ${MAC_PLIST}"
    echo
    echo -e "${BOLD}Next steps (macOS):${RESET}"
    echo "  1. Edit your coordinates:  ${MAC_DATA_DIR}/birdnet.conf  (LATITUDE/LONGITUDE)"
    echo "     and BNB_STATION_LAT/LON in ${MAC_PLIST}"
    echo "  2. Start it:  launchctl load -w \"${MAC_PLIST}\""
    echo "  3. The first microphone access shows a permission prompt — approve it under"
    echo "     System Settings → Privacy & Security → Microphone."
    echo "  4. Open http://localhost:8502  (the BirdNET+ model downloads on first run)."
    echo "  Uninstall any time with:  ./uninstall.sh   (or --purge to remove data too)"
}

macos_install() {
    echo -e "${BOLD}BirdNet-Behavior — macOS (Apple Silicon)${RESET}"
    if [ "$(id -u)" -eq 0 ]; then
        fatal "Do NOT run the macOS install with sudo — the launchd LaunchAgent is per-user. Re-run without sudo."
    fi
    if [ "$(uname -m)" != "arm64" ]; then
        warn "This Mac is $(uname -m), not arm64 — there is no prebuilt ONNX Runtime for Intel macOS. The source build below may still work."
    fi

    mac_brew_dep ffmpeg "needed for microphone capture (avfoundation) and RTSP streams."

    local version asset url tmp inner bindst
    # tail -1 drops resolve_version's progress line; `|| true` + 2>/dev/null mean
    # "no releases yet" yields an empty version (→ friendly source guidance below)
    # rather than a hard fatal.
    version="$(resolve_version 2>/dev/null | tail -1 || true)"
    if [ -n "${version}" ]; then
        asset="${BINARY_NAME}-${version}-aarch64-apple-darwin.tar.gz"
        url="https://github.com/${REPO}/releases/download/v${version}/${asset}"
    fi

    if [ -n "${version}" ] && curl -fsIL "${url}" >/dev/null 2>&1; then
        info "Downloading prebuilt macOS binary (v${version})…"
        tmp="$(mktemp -d)"
        download_large "${url}" "${tmp}/${asset}" "${asset}"
        tar -xzf "${tmp}/${asset}" -C "${tmp}"
        inner="${tmp}/${BINARY_NAME}-${version}-aarch64-apple-darwin/${BINARY_NAME}"
        if [ -w "/opt/homebrew/bin" ]; then bindst="/opt/homebrew/bin"; else bindst="${HOME}/.local/bin"; mkdir -p "${bindst}"; fi
        install -m 0755 "${inner}" "${bindst}/${BINARY_NAME}"
        rm -rf "${tmp}"
        success "Installed ${bindst}/${BINARY_NAME}"
        case ":${PATH}:" in *":${bindst}:"*) ;; *) warn "${bindst} is not on your PATH — add it or call the binary by full path." ;; esac
        macos_setup_config_and_agent "${bindst}/${BINARY_NAME}"
    else
        mac_brew_dep cmake "needed to compile the bundled libduckdb when building from source."
        local script_dir
        script_dir="$(cd "$(dirname "$0")" 2>/dev/null && pwd || echo "")"
        if [ -n "${script_dir}" ] && [ -f "${script_dir}/Cargo.toml" ] && [ -d "${script_dir}/crates" ]; then
            # Already inside a source checkout — offer to build it right here
            # instead of telling the user to clone what they're running from.
            warn "No prebuilt macOS binary${version:+ for v${version}} is published yet — but you're in a source checkout."
            command -v cargo >/dev/null 2>&1 || fatal "cargo not found — install Rust from https://rustup.rs, then re-run."
            local ans="n"
            if [ -t 0 ]; then read -rp "  Build it now with 'cargo build --release --features analytics' (~6 min)? [Y/n] " ans </dev/tty 2>/dev/null || ans="y"; fi
            case "$ans" in
                n|N|no|NO)
                    echo "  Build later:  cargo build --release --features analytics && bash packaging/macos/verify-macos.sh" ;;
                *)
                    info "Building (this takes a few minutes)…"
                    ( cd "${script_dir}" && cargo build --release --features analytics ) || fatal "build failed — see the cargo output above."
                    success "Build complete."
                    macos_setup_config_and_agent "${script_dir}/target/release/${BINARY_NAME}" ;;
            esac
        else
            warn "No prebuilt macOS binary${version:+ for v${version}} is published yet — build from source (one time, ~6 min):"
            cat <<EOF

    git clone https://github.com/${REPO}.git
    cd BirdNet-Behavior
    cargo build --release --features analytics
    bash packaging/macos/verify-macos.sh   # verifies the build + writes a ready LaunchAgent

  (A Homebrew formula is planned so this becomes a one-line 'brew install'.)
EOF
        fi
    fi
}

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------

SUBCOMMAND=""

usage() {
    cat <<EOF
BirdNet-Behavior installer

Linux / Raspberry Pi (installs a systemd service, so it needs root):
  curl -fsSL https://raw.githubusercontent.com/${REPO}/main/install.sh | sudo bash
  curl -fsSL https://raw.githubusercontent.com/${REPO}/main/install.sh | sudo bash -s -- --version X.Y.Z

From a saved copy of this script:
  sudo bash install.sh [--version X.Y.Z]
  sudo bash install.sh uninstall

Options:
  -v, --version X.Y.Z   Install a specific release (default: latest stable).
                        The VERSION environment variable is still honoured too.
      --noninteractive  Don't prompt; auto-detect audio and leave location
                        unset (also implied by BIRDNET_NONINTERACTIVE=1 or no TTY).
  -h, --help            Show this help and exit.

Avoid  sudo bash <(curl ...)  — sudo closes the process-substitution file
descriptor on the way to root, so the script never loads. Use the pipe above.
EOF
}

parse_args() {
    while [ "$#" -gt 0 ]; do
        case "$1" in
            -v | --version)
                [ "$#" -ge 2 ] || fatal "--version needs a value, e.g. --version 0.5.1"
                VERSION="$2"
                shift 2
                ;;
            --version=*)
                VERSION="${1#*=}"
                shift
                ;;
            --noninteractive | --non-interactive)
                BIRDNET_NONINTERACTIVE=1
                shift
                ;;
            -h | --help)
                usage
                exit 0
                ;;
            uninstall)
                SUBCOMMAND="uninstall"
                shift
                ;;
            --)
                shift
                ;;
            -*)
                fatal "Unknown option: $1  (run with --help for usage)."
                ;;
            *)
                fatal "Unexpected argument: $1  (run with --help for usage)."
                ;;
        esac
    done
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

main() {
    parse_args "$@"

    # Prompt only with a real terminal to read from. Under `curl ... | sudo bash`
    # stdin is the pipe, but stdout (fd 1) and /dev/tty are still the user's
    # terminal — so gate on those, not on stdin.
    if [ "${BIRDNET_NONINTERACTIVE:-0}" != "1" ] && [ -t 1 ] && [ -r /dev/tty ]; then
        INTERACTIVE=1
    fi

    echo -e "${BOLD}BirdNet-Behavior Installer${RESET}"
    echo "  Repository: https://github.com/${REPO}"
    echo

    # macOS is not systemd — dispatch to the per-user launchd path before any
    # root check, download, or filesystem change, so a Mac user never gets a
    # half-finished Linux install.
    if [ "$(uname -s)" = "Darwin" ]; then
        if [ "${SUBCOMMAND}" = "uninstall" ]; then
            warn "On macOS, uninstall with:  ./uninstall.sh   (it handles the launchd LaunchAgent)."
            exit 0
        fi
        macos_install
        exit 0
    fi

    if [ "${SUBCOMMAND}" = "uninstall" ]; then
        do_uninstall
        exit 0
    fi

    require_root

    local arch version
    arch="$(detect_arch)"
    check_glibc
    version="$(resolve_version)"

    info "Arch: ${arch}, Version: ${version}"

    # Upgrade-safe: stop a running service before swapping the binary. You
    # cannot overwrite a running executable in place (ETXTBSY), and a plain
    # `systemctl start` on an already-running unit would not load the new
    # binary. Record that it was running so maybe_start_service restarts it.
    if systemctl is-active --quiet birdnet-behavior.service 2>/dev/null; then
        SERVICE_WAS_RUNNING=1
        info "Stopping the running service to upgrade the binary safely…"
        systemctl stop birdnet-behavior.service || true
    fi

    install_binary "${version}" "${arch}"
    create_directories
    setup_tmpfs_streaming
    download_model
    prompt_station_settings
    write_config
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
