#!/usr/bin/env bash
# =============================================================================
#  install.sh — GENERATED FILE. DO NOT EDIT.
#
#  This file is assembled from installer/lib/*.sh by installer/build.sh.
#  To change the installer, edit the relevant module under installer/lib/ and
#  run `installer/build.sh`. CI verifies this file stays in sync.
# =============================================================================

# ===== installer/lib/00-usage.sh =====
# install.sh — BirdNet-Behavior installer for Raspberry Pi and x86_64 Linux
#
# Usage (Linux / Raspberry Pi — installs a systemd service, so it needs root):
#   curl -fsSL https://raw.githubusercontent.com/tomtom215/BirdNet-Behavior/main/install.sh | sudo bash
#   # pin a specific version (the `-s --` passes the args through to the script):
#   curl -fsSL https://raw.githubusercontent.com/tomtom215/BirdNet-Behavior/main/install.sh | sudo bash -s -- --version 0.5.1
#   # from a saved copy:
#   sudo bash install.sh [--version 0.5.1]
#
# When an existing install is detected the script offers update / repair /
# reinstall / uninstall. You can also pick one explicitly:
#   sudo bash install.sh update       # swap in the latest binary, keep settings
#   sudo bash install.sh repair       # fix dirs/permissions + rewrite the unit
#   sudo bash install.sh reinstall    # re-download and rewrite everything
#   sudo bash install.sh uninstall    # remove the software, keep data
#
# Do NOT use `sudo bash <(curl ...)`: process substitution hands bash a file
# descriptor owned by your user, and sudo closes it crossing to root, so the
# script disappears ("/dev/fd/63: No such file or directory"). Use the pipe.
#
# macOS (Apple Silicon) sets up a per-user launchd agent instead — run without sudo.
#
# What this script does on a fresh install:
#   1. Pre-flight checks (architecture, glibc, required tools, free disk)
#   2. Downloads + checksum-verifies the pre-built binary from GitHub Releases
#   3. Creates configuration, data, and recording directories
#   4. Installs a hardened systemd service unit (birdnet-behavior.service)
#   5. Optionally prompts for ALSA device / RTSP URL / location
#   6. Post-install validation (binary, unit, directories, doctor, port)
#
# Every step is idempotent — re-running the script is always safe.
#
# Offline / air-gapped install (no internet on the target):
#   BIRDNET_BINARY_TARBALL=/path/to/birdnet-behavior-<ver>-<target>.tar.gz \
#     BIRDNET_SKIP_MODEL=1 sudo -E bash install.sh
#   Installs the binary from a tarball you already downloaded (skips the GitHub
#   fetch + checksum round-trip — you vouch for the local file), and skips the
#   ~541 MB model pull. Place the model in <data>/models and restart afterwards.
#
# Environment overrides (all optional):
#   BIRDNET_BINARY_TARBALL=PATH  install the binary from a local tarball (offline)
#   BIRDNET_SKIP_MODEL=1         do not download the model (stage it out-of-band)
#   BIRDNET_NONINTERACTIVE=1     never prompt; take env/config/defaults
#   BIRDNET_LISTEN=HOST:PORT     dashboard bind address (default 0.0.0.0:8502)
#   BIRDNET_SKIP_GLIBC_CHECK=1   bypass the glibc floor check (built from source)
#
# Without systemd (a container, chroot, or staged image) the script still lays
# down the binary, config, and unit file, then tells you how to enable it on a
# real host — it does not abort.
#
# Requirements: curl or wget, tar, sha256sum (systemd recommended, not required)

# ===== installer/lib/10-config.sh =====
# ---------------------------------------------------------------------------
# Global configuration and shared state
# ---------------------------------------------------------------------------
set -euo pipefail

REPO="tomtom215/BirdNet-Behavior"
BINARY_NAME="birdnet-behavior"
INSTALL_DIR="/usr/local/bin"
# Rendered operator manual (mdBook), bundled in the release tarball and served
# at /help/* via BNB_HELP_DIR. A read-only system path so the sandboxed service
# (ProtectSystem=strict) can read it without any ReadWritePaths grant.
HELP_DIR="/usr/local/share/birdnet-behavior/help"
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
SERVICE_NAME="birdnet-behavior.service"
SERVICE_USER="${SUDO_USER:-${USER:-$(id -un)}}"
# Bind to all interfaces by default so the dashboard is reachable on the LAN out
# of the box (a localhost-only default left non-technical users staring at
# "connection refused"). Safe because only the /admin panel is gated by a
# password — viewing is open — and a fresh install auto-generates that admin
# password. Override with BIRDNET_LISTEN=, the config file, or the prompt;
# restrict to this host with 127.0.0.1:8502.
LISTEN_ADDR="${BIRDNET_LISTEN:-0.0.0.0:8502}"

# Interactive onboarding state. INTERACTIVE is decided in main(); the *_VALUE
# vars hold answers from prompt_station_settings and are baked into the config
# by write_config (empty = keep the commented example in place).
INTERACTIVE=0
ALSA_CARD_VALUE=""
RTSP_URL_VALUE=""
LATITUDE_VALUE=""
LONGITUDE_VALUE=""
CADDY_USER_VALUE=""
CADDY_PWD_VALUE=""
# Set by ensure_admin_password when it auto-generates one, so print_summary can
# show it to the operator exactly once.
GENERATED_ADMIN_PASSWORD=""

# What main() is doing — purely for user-facing messages (install/update/
# repair/reinstall). Set by main()/the subcommand dispatch.
MODE="install"

# Selected action. "" means "decide from args + whether an install exists".
SUBCOMMAND=""

# Minimum glibc the prebuilt binaries link against (pyke's ONNX Runtime needs
# glibc >= 2.38; Ubuntu 24.04, where we build, ships 2.39). Set
# BIRDNET_SKIP_GLIBC_CHECK=1 to bypass (e.g. you built from source).
REQUIRED_GLIBC="2.39"

# Rough free-space floor for a fresh install: ~541 MB model + binary + DB
# headroom. Below this we warn (model download would otherwise fail mid-way
# with a less obvious error).
REQUIRED_FREE_MB=900

# Set to 1 by main() when an already-running service is stopped for an upgrade,
# so we know to restart it afterwards rather than leave it down.
SERVICE_WAS_RUNNING=0

# BirdNET+ V3.0 model files. FP32 ONNX (~541 MB): the same model BirdNET-Pi
# uses, arch-independent and working on every platform.
#
# Primary origin is a stable, arch-independent GitHub release shared by all app
# releases (uploaded once, never re-pushed per patch), so a fresh install pulls
# the binary AND the model from the same host (GitHub) and is offline-capable
# after that single fetch. Zenodo is the upstream source and the fallback when
# the GitHub asset is absent (older app releases) or unreachable.
#
# Both files are verified against the pinned sha256 below regardless of which
# origin served them — these hashes are the integrity root of trust (they live
# in version-controlled, provenance-attested source), so a corrupted or tampered
# download from either host is rejected. The same hashes are published as the
# `SHA256SUMS` asset of the models release; publish-model.yml cross-checks the
# Zenodo bytes against these values before it uploads, so the two never drift.
MODEL_FILE="BirdNET+_V3.0-preview3_Global_11K_FP32.onnx"
LABELS_FILE="BirdNET+_V3.0-preview3_Global_11K_Labels.csv"
MODEL_SHA256="2a0f9efba1a98e3193ad3dfcb8323116a7de88e39545f3619a7ea46e3bb7d743"
LABELS_SHA256="8124b0ea2d187104c5e2cd95a0f937165647e20349c8fd34d4d5ef991821f8f0"

# Primary origin: the stable GitHub models release (one per model version).
MODEL_RELEASE_TAG="models-v3.0-preview3"
MODEL_GH_BASE="https://github.com/${REPO}/releases/download/${MODEL_RELEASE_TAG}"

# Fallback origin: Zenodo. The API /content endpoint handles the + in the
# filenames correctly and needs no login.
ZENODO_RECORD="18247420"
ZENODO_API="https://zenodo.org/api/records/${ZENODO_RECORD}/files"

# ---------------------------------------------------------------------------
# BirdNET Geomodel v3.0.2 — the species occurrence ("range") filter.
#
# Separate from the classifier above, and versioned separately: the classifier
# says *what* it heard, the geomodel says which species plausibly occur at this
# latitude/longitude in this week of the year. Without it the station keeps
# every one of the classifier's ~11 560 species as a candidate wherever it is,
# which is how a garden in Berlin reports birds that have never left Peru.
#
# The two do NOT score the same species list — the geomodel covers 12 012
# species across birds, mammals, insects, amphibians and reptiles — so its own
# label file ships beside it and is what maps one list onto the other. Both are
# required; the station refuses a model it cannot align rather than reading one
# list's index into the other.
#
# FP32 rather than FP16: both are genuine upstream artifacts and agree to
# within one species in ~300 at the default threshold, but FP32 loads about
# twice as fast (no FP16→FP32 cast nodes for the CPU execution provider) and
# upstream marks it the recommended variant. 14 MB against the classifier's
# 541 MB is not a size worth optimising.
#
# Origins mirror the classifier's: our own models release first (same host as
# the binary), then the upstream birdnet-team release. Both are verified
# against the sha256 pinned here before the bytes are accepted.
#
# Licence: the geomodel weights are CC BY-SA 4.0 (Stefan Kahl, K. Lisa Yang
# Center for Conservation Bioacoustics) with prohibited uses covering poaching
# and military applications — see MODEL_LICENSE.txt in the upstream release.
# Redistribution is permitted with attribution; that is what the mirror does.
GEOMODEL_VERSION="v3.0.2"
GEOMODEL_FILE="BirdNET+_Geomodel_V3.0.2_Global_12K_FP32.onnx"
GEOMODEL_LABELS_FILE="BirdNET+_Geomodel_V3.0.2_Global_12K_Labels.txt"
GEOMODEL_SHA256="b151f680a47de5371f39b3df129aea5946ac6baa039582274f833b42eaf992ea"
GEOMODEL_LABELS_SHA256="c15818db07e55978d909a9bcd916cd0615b0183f789227d9516059151787c784"

# Primary origin: our models release (the same one the classifier comes from,
# so a fresh install still contacts a single host).
GEOMODEL_GH_BASE="${MODEL_GH_BASE}"
# Fallback origin: the upstream release the mirror is taken from.
GEOMODEL_UPSTREAM_BASE="https://github.com/birdnet-team/geomodel/releases/download/${GEOMODEL_VERSION}"

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

# ===== installer/lib/20-log.sh =====
# ---------------------------------------------------------------------------
# Logging and interactive prompt helpers
# ---------------------------------------------------------------------------

# All logging goes to stderr so that stdout is reserved for a function's return
# value. resolve_version/detect_arch return via `echo`, and callers capture them
# with $(...) — a log line on stdout would be swallowed into the value (e.g. a
# version of "[INFO] Querying…\n0.5.1", which then corrupts the download URL).
info()    { echo -e "${BLUE}[INFO]${RESET}  $*" >&2; }
success() { echo -e "${GREEN}[OK]${RESET}    $*" >&2; }
warn()    { echo -e "${YELLOW}[WARN]${RESET}  $*" >&2; }
error()   { echo -e "${RED}[ERROR]${RESET} $*" >&2; }
fatal()   { error "$*"; exit 1; }

# A deliberately LOUD, boxed warning for opt-in bypass flags (BIRDNET_SKIP_*).
# Like the helpers above it writes to stderr — stdout stays reserved for a
# function's captured return value — but it draws an ASCII `!!' box so it stands
# out from the routine stream of [WARN] lines and, crucially, survives the
# non-TTY case where the colour codes are stripped to empty strings
# (10-config.sh): in an automated/piped install a lone [WARN] is easy to miss,
# and these bypasses fail later with a cryptic downstream error. Each argument
# is rendered as one line inside the box.
loud_warn() {
    local line
    {
        echo -e "${BOLD}${RED}!! ========================================================= !!${RESET}"
        for line in "$@"; do
            echo -e "${BOLD}${RED}!!${RESET} ${YELLOW}${line}${RESET}"
        done
        echo -e "${BOLD}${RED}!! ========================================================= !!${RESET}"
    } >&2
}

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

# Like ask, but does not echo the input — for passwords. Reads from /dev/tty.
ask_secret() {
    local prompt="$1" reply
    printf '%s: ' "${prompt}" >/dev/tty
    read -rs reply </dev/tty || reply=""
    printf '\n' >/dev/tty
    printf '%s' "${reply}"
}

# ===== installer/lib/30-platform.sh =====
# ---------------------------------------------------------------------------
# Privilege, architecture, and glibc preflight
# ---------------------------------------------------------------------------

# ---------------------------------------------------------------------------
# Package manager abstraction
#
# Raspberry Pi OS and Debian are the primary targets, but the binary is a plain
# x86_64/aarch64 ELF and people do run it on Fedora, Arch and openSUSE. Every
# auto-install path used to be gated on `command -v apt-get`, so on those
# distros the installer printed an apt command that cannot run — advice that is
# worse than silence, because it looks authoritative.
#
# Package names were checked against real containers rather than assumed
# (fedora:41, archlinux, opensuse/tumbleweed): alsa-utils, qrencode and
# util-linux carry the same name on all four. ffmpeg is the sole exception —
# Fedora's main repositories ship it as `ffmpeg-free`, with the unencumbered
# `ffmpeg` living in RPM Fusion, which we will not enable on an operator's box.
# ---------------------------------------------------------------------------

# apt | dnf | pacman | zypper | "" when none is recognised. Set by
# detect_pkg_mgr, which is idempotent, so callers can just call it first.
PKG_MGR=""

detect_pkg_mgr() {
    [ -n "${PKG_MGR}" ] && return 0
    if command -v apt-get &>/dev/null; then
        PKG_MGR="apt"
    elif command -v dnf &>/dev/null; then
        PKG_MGR="dnf"
    elif command -v pacman &>/dev/null; then
        PKG_MGR="pacman"
    elif command -v zypper &>/dev/null; then
        PKG_MGR="zypper"
    fi
    return 0
}

# Translate a generic package name into this distro's name for it.
pkg_name_for() {
    detect_pkg_mgr
    case "$1:${PKG_MGR}" in
        ffmpeg:dnf) echo "ffmpeg-free" ;;
        *)          echo "$1" ;;
    esac
}

# Install one package. Quiet on success; returns non-zero when the package
# manager is unknown or the install failed, and every caller treats that as
# "warn and carry on" rather than as fatal.
pkg_install() {
    detect_pkg_mgr
    local pkg
    pkg="$(pkg_name_for "$1")"
    case "${PKG_MGR}" in
        apt)
            apt-get install -y "${pkg}" &>/dev/null \
                || { apt-get update &>/dev/null && apt-get install -y "${pkg}" &>/dev/null; }
            ;;
        dnf)
            dnf install -y "${pkg}" &>/dev/null
            ;;
        pacman)
            # Deliberately NOT `-Syu`: a full system upgrade is not something an
            # application installer should trigger. `-S` alone fails on a box
            # whose package database was never synced, so refresh (`-Sy`) and
            # retry — accepting the documented partial-upgrade caveat, which is
            # the lesser evil against upgrading someone's whole system.
            pacman -S --noconfirm --needed "${pkg}" &>/dev/null \
                || { pacman -Sy --noconfirm &>/dev/null \
                     && pacman -S --noconfirm --needed "${pkg}" &>/dev/null; }
            ;;
        zypper)
            zypper --non-interactive --gpg-auto-import-keys install --no-recommends "${pkg}" &>/dev/null
            ;;
        *)
            return 1
            ;;
    esac
}

# The command an operator should run by hand when pkg_install could not.
# Accepts one or more generic package names.
pkg_install_hint() {
    detect_pkg_mgr
    local pkgs="" p
    for p in "$@"; do
        pkgs="${pkgs:+${pkgs} }$(pkg_name_for "${p}")"
    done
    case "${PKG_MGR}" in
        apt)    echo "sudo apt-get install -y ${pkgs}" ;;
        dnf)    echo "sudo dnf install -y ${pkgs}" ;;
        pacman) echo "sudo pacman -S --needed ${pkgs}" ;;
        zypper) echo "sudo zypper install ${pkgs}" ;;
        *)      echo "install ${pkgs} with your distribution's package manager" ;;
    esac
}

# Whether systemd is the running init, so `systemctl` calls will actually work.
#
# `systemctl` can be present on a system where systemd is NOT PID 1 — minimal
# containers, chroots, WSL1, some CI runners — and there every systemctl call
# fails. `/run/systemd/system` is systemd's own "I am running" marker, so this
# is the canonical guard. When it returns false the installer writes the unit
# but skips enable/start, degrading cleanly instead of aborting (see
# install_service / maybe_start_service).
has_systemd() {
    command -v systemctl >/dev/null 2>&1 && [ -d /run/systemd/system ]
}

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

# The prebuilt release binaries are built on Ubuntu 24.04 (glibc 2.39) because
# pyke's prebuilt ONNX Runtime requires glibc >= 2.38. On an older system
# (notably Raspberry Pi OS Bookworm / Debian 12, glibc 2.36) the binary loads
# but dies with "version `GLIBC_2.39' not found". Catch that here, before we
# download 540 MB of model, and point the user at a path that actually works.
detect_glibc_version() {
    # Prefer getconf (prints "glibc 2.36"); fall back to `ldd --version`.
    local v
    v="$(getconf GNU_LIBC_VERSION 2>/dev/null | awk '{print $2}')"
    if [ -z "${v}" ]; then
        # `awk 'NR==1'` rather than `head -1`: awk reads its input to the end,
        # so the producer is never left writing into a closed pipe. `ldd
        # --version` prints five lines and `head -1` quits after one, which
        # under `set -o pipefail` made this assignment return 141 and `set -e`
        # abort the installer with no message at all — measured at 3 failures
        # in 200 runs. Only systems where `getconf GNU_LIBC_VERSION` is empty
        # reach this line, which is why it was never seen on Debian.
        v="$(ldd --version 2>/dev/null | awk 'NR==1' | grep -oE '[0-9]+\.[0-9]+' | tail -1)"
    fi
    echo "${v}"
}

check_glibc() {
    if [ "${BIRDNET_SKIP_GLIBC_CHECK:-0}" = "1" ]; then
        loud_warn "BIRDNET_SKIP_GLIBC_CHECK=1 — glibc compatibility check BYPASSED." \
                  "If this system's glibc is older than ${REQUIRED_GLIBC}, the daemon will" \
                  "crash at startup with a 'GLIBC_${REQUIRED_GLIBC} not found' error." \
                  "Unset BIRDNET_SKIP_GLIBC_CHECK to re-enable the check."
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
    if [ "$(printf '%s\n%s\n' "${REQUIRED_GLIBC}" "${current}" | sort -V | awk 'NR==1')" = "${REQUIRED_GLIBC}" ]; then
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

# ===== installer/lib/40-download.sh =====
# ---------------------------------------------------------------------------
# Download helpers (curl or wget) and release-version resolution
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
#   - A definitive HTTP 404 fails immediately (no retries) so a missing asset
#     falls through to the next source at once instead of stalling ~10 s on
#     five back-off retries — `fetch_verified_model` then tries Zenodo. We do
#     NOT pass --retry-all-errors (which would retry the 404); curl's default
#     --retry already covers the transient cases (timeouts, 5xx, 429), and
#     --retry-connrefused keeps a slow-to-wake CDN resilient.
#   - Leave the partial file in place on failure so the next run can resume.
download_large() {
    local url="$1"
    local dest="$2"
    local name="${3:-${dest##*/}}"
    info "  Fetching ${name}…"
    if command -v curl &>/dev/null; then
        # -C - : resume; -# : progress bar.
        curl -fL -C - -# \
            --retry 5 --retry-delay 2 --retry-connrefused --retry-max-time 600 \
            --connect-timeout 30 \
            -o "${dest}" "${url}"
    elif command -v wget &>/dev/null; then
        # -c : resume; --show-progress to stderr; tolerate transient failures.
        wget -c --tries=5 --waitretry=2 --timeout=30 --show-progress -O "${dest}" "${url}"
    else
        fatal "Neither curl nor wget is available. Please install one and retry."
    fi
}

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
        ver="$(grep '"tag_name"' "${tmp}" | sed -E 's/.*"v?([^"]+)".*/\1/' | awk 'NR==1')"
        rm -f "${tmp}"
        if [ -n "${ver}" ]; then
            echo "${ver}"
            return
        fi
    fi
    rm -f "${tmp}"
    fatal "Could not determine latest release version. Pass --version x.y.z (or set VERSION=x.y.z) to install a specific version."
}

# ===== installer/lib/45-preflight.sh =====
# ---------------------------------------------------------------------------
# Pre-install checks and installation-state detection
#
# These run before any download or filesystem change so a doomed install fails
# fast with an actionable message instead of half-way through a 541 MB pull.
# ---------------------------------------------------------------------------

# Verify the tools the installer itself depends on are present. A hard-missing
# tool is fatal (we cannot continue); optional tools only degrade a feature.
check_required_tools() {
    local missing=()

    # A downloader is mandatory.
    command -v curl &>/dev/null || command -v wget &>/dev/null \
        || missing+=("curl or wget")

    local t
    for t in tar sha256sum install mkdir awk grep sed; do
        command -v "${t}" &>/dev/null || missing+=("${t}")
    done

    if [ "${#missing[@]}" -gt 0 ]; then
        error "Missing required tool(s): ${missing[*]}"
        fatal "Install the missing package(s) and re-run:  $(pkg_install_hint coreutils tar curl)"
    fi

    # systemd is the normal service manager, but the install can still lay down
    # the binary, config, and unit file without it (containers, chroots, an
    # air-gapped stage-then-boot flow). Degrade with a clear note rather than
    # hard-failing — install_service / maybe_start_service skip the systemctl
    # steps when has_systemd is false.
    if has_systemd; then
        success "systemd is running — the service will be enabled and started."
    else
        warn "systemd is not running here — the unit will be written but not enabled/started."
        warn "  On a systemd host: sudo systemctl daemon-reload && sudo systemctl enable --now birdnet-behavior"
    fi

    # Soft dependencies — note them but keep going.
    command -v getent  &>/dev/null || warn "getent not found — falling back to default data paths."
    command -v findmnt &>/dev/null || warn "findmnt not found — cannot confirm /tmp is tmpfs."
    # arecord(1) is not optional for a microphone station: it is the capture
    # backend the daemon spawns, and it is also what the ALSA auto-detect below
    # reads the card list from. Install it here — before onboarding — so the
    # detection has something to detect. ensure_capture_backend() re-checks it
    # once the RTSP/ALSA choice is settled; this earlier pass is what stops a
    # missing alsa-utils from silently yielding "no capture device found".
    if ! command -v arecord &>/dev/null; then
        info "arecord not found — installing alsa-utils for microphone capture…"
        pkg_install alsa-utils \
            || warn "Could not install alsa-utils ($(pkg_install_hint alsa-utils)) — microphone auto-detect will find nothing."
    fi
    # qrencode is optional: it lets the final summary print a scannable QR of the
    # dashboard URL so a phone can open it without anyone typing an IP. Best-effort
    # and silent — a missing QR helper must never fail or slow the install.
    if ! command -v qrencode &>/dev/null; then
        pkg_install qrencode || true
    fi
    success "Required tools present."
}

# Free megabytes on the filesystem backing PATH (walks up to the nearest
# existing ancestor, since the target dir may not exist yet). Prints a number.
free_mb_for_path() {
    local p="$1"
    while [ ! -d "${p}" ] && [ "${p}" != "/" ]; do
        p="$(dirname "${p}")"
    done
    df -Pk "${p}" 2>/dev/null | awk 'NR==2 {printf "%d", $4/1024}'
}

# Warn (don't fail) when the data filesystem is tight. Skipped when the model
# is already present, since that is the only large download.
check_disk_space() {
    if [ -f "${MODEL_DIR}/${MODEL_FILE}" ]; then
        return 0
    fi
    local free_mb
    free_mb="$(free_mb_for_path "${DATA_DIR}")"
    if [ -z "${free_mb}" ]; then
        warn "Could not determine free disk space for ${DATA_DIR}; continuing."
        return 0
    fi
    if [ "${free_mb}" -lt "${REQUIRED_FREE_MB}" ]; then
        warn "Only ${free_mb} MB free on the data filesystem (${DATA_DIR})."
        warn "The BirdNET model alone is ~541 MB; free up space or the download may fail."
    else
        success "Disk space OK (${free_mb} MB free for ${DATA_DIR})."
    fi
}

# Guarantee one capture tool is on PATH, installing it when apt-get is
# available. $1 = command, $2 = package, $3 = what breaks without it.
#
# Returns 0 if the tool ends up present, 1 if the operator has to act. Both
# outcomes are reported; neither is fatal, because the rest of the install
# (binary, config, unit, model) is still worth completing.
ensure_capture_tool() {
    local tool="$1" pkg="$2" purpose="$3"

    if command -v "${tool}" &>/dev/null; then
        success "${tool} present — ${purpose} OK."
        return 0
    fi

    detect_pkg_mgr
    warn "${purpose} needs ${tool}(1), which is not installed (package: $(pkg_name_for "${pkg}"))."
    if [ -n "${PKG_MGR}" ]; then
        info "Installing $(pkg_name_for "${pkg}")…"
        if pkg_install "${pkg}"; then
            success "$(pkg_name_for "${pkg}") installed."
            return 0
        fi
        warn "Automatic install failed."
    fi
    warn "Install it, then restart the service:"
    if [ -n "${PKG_MGR}" ]; then
        # A real command, so the two halves can be chained and pasted as one.
        warn "  $(pkg_install_hint "${pkg}") && sudo systemctl restart birdnet-behavior"
    else
        # Prose, not a command — chaining it with && would produce something
        # that looks runnable and is not.
        warn "  $(pkg_install_hint "${pkg}")"
        warn "  then: sudo systemctl restart birdnet-behavior"
    fi
    return 1
}

# Make sure the capture backend this station will actually use is installed.
# Called by the install/repair flows AFTER the config is written/known, so the
# RTSP_URL / ALSA choice is settled.
#
# The daemon shells out to one of two tools (`audio::capture::manager`): ffmpeg
# for an RTSP source, arecord for a local microphone. Only ffmpeg used to be
# ensured here, on the reasoning that "an ALSA microphone needs no ffmpeg" —
# true, but it needs arecord, and nothing installed that either. Raspberry Pi OS
# ships alsa-utils so the gap stayed invisible; on a minimal Debian it produces
# a station that installs cleanly, starts cleanly, and records nothing, with the
# capture failure buried in the supervisor's restart loop.
ensure_capture_backend() {
    local rtsp=0
    if [ -n "${RTSP_URL_VALUE}" ]; then
        rtsp=1
    elif [ -f "${CONFIG_FILE}" ] \
        && grep -qE '^[[:space:]]*RTSP_URL[[:space:]]*=[[:space:]]*[^[:space:]#]' "${CONFIG_FILE}"; then
        rtsp=1
    fi

    # The `|| true` on both calls is load-bearing, not decoration: this script
    # runs under `set -e`, ensure_capture_tool returns non-zero when the
    # operator has to install the tool by hand, and without the guard that
    # would abort the whole install over a warning it has already printed.
    if [ "${rtsp}" = 1 ]; then
        ensure_capture_tool ffmpeg ffmpeg "RTSP capture" || true
        return 0
    fi

    ensure_capture_tool arecord alsa-utils "microphone capture" || true

    # A microphone station needs ffmpeg too — not to capture, but to *listen*.
    # `GET /stream`, behind the dashboard's Listen -> Live tab, shells out to
    # ffmpeg for every source kind including plain ALSA. Ensuring it only for
    # RTSP left the commonest station of all (Linux + USB mic) with a tab that
    # returned 500 on every request, an ENOENT buried in the journal, and a
    # `--doctor` that reported the station perfectly healthy because its own
    # ffmpeg check was gated on the same RTSP-only condition.
    #
    # Kept non-fatal: unlike the capture tool above, a station without this
    # still records and detects exactly as it should, so a failed package
    # install must not abort the run — it costs live listening, nothing more.
    ensure_capture_tool ffmpeg ffmpeg "live audio streaming (Listen -> Live)" || true
}

# Detect what — if anything — is already installed, into globals the rest of
# the script reads. Safe to call before require_root has run.
HAVE_BINARY=0
HAVE_SERVICE=0
HAVE_CONFIG=0
INSTALLED_VERSION=""
EXISTING_INSTALL=0

detect_existing_install() {
    HAVE_BINARY=0; HAVE_SERVICE=0; HAVE_CONFIG=0; INSTALLED_VERSION=""; EXISTING_INSTALL=0
    if [ -x "${INSTALL_DIR}/${BINARY_NAME}" ]; then
        HAVE_BINARY=1
        INSTALLED_VERSION="$("${INSTALL_DIR}/${BINARY_NAME}" --version 2>/dev/null | awk '{print $NF}' || true)"
    fi
    [ -f "${SERVICE_FILE}" ] && HAVE_SERVICE=1
    [ -f "${CONFIG_FILE}" ]  && HAVE_CONFIG=1
    if [ "${HAVE_BINARY}" = 1 ] || [ "${HAVE_SERVICE}" = 1 ]; then
        EXISTING_INSTALL=1
    fi
}

# One-line human summary of the detected install (for the menu / messages).
describe_existing_install() {
    local parts=()
    [ "${HAVE_BINARY}" = 1 ]  && parts+=("binary${INSTALLED_VERSION:+ v${INSTALLED_VERSION}}")
    [ "${HAVE_SERVICE}" = 1 ] && parts+=("service unit")
    [ "${HAVE_CONFIG}" = 1 ]  && parts+=("config")
    if [ "${#parts[@]}" -eq 0 ]; then
        echo "none"
        return
    fi
    local out="" p
    for p in "${parts[@]}"; do
        out="${out:+${out}, }${p}"
    done
    echo "${out}"
}

# ===== installer/lib/50-binary.sh =====
# ---------------------------------------------------------------------------
# Download and install the binary
#
# Release artifacts are gzipped tarballs of the form
#   birdnet-behavior-<version>-<target>.tar.gz
# containing a single top-level directory with the stripped binary alongside
# README, LICENSE, LICENSE-UPSTREAM, CHANGELOG, this script, and (since 0.6.0)
# a help/ directory holding the rendered operator manual served at /help/*. A
# single SHA256SUMS file is attached to each GitHub Release for verification.
# ---------------------------------------------------------------------------

install_binary() {
    local version="$1"
    local arch="$2"

    if [ -x "${INSTALL_DIR}/${BINARY_NAME}" ]; then
        local old_ver
        old_ver="$("${INSTALL_DIR}/${BINARY_NAME}" --version 2>/dev/null | awk '{print $NF}' || true)"
        if [ -n "${old_ver}" ]; then
            info "Existing install detected (v${old_ver}) — installing v${version}."
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

    # Air-gapped / offline install: BIRDNET_BINARY_TARBALL points at a release
    # tarball already on disk (downloaded on another machine, or shipped on
    # media), so the install needs no network for the binary. The operator
    # vouches for a local file they placed themselves, so we skip the
    # SHA256SUMS round-trip and verify only the archive's internal layout.
    if [ -n "${BIRDNET_BINARY_TARBALL:-}" ]; then
        if [ ! -f "${BIRDNET_BINARY_TARBALL}" ]; then
            fatal "BIRDNET_BINARY_TARBALL=${BIRDNET_BINARY_TARBALL} is not a file."
        fi
        info "Using local binary tarball ${BIRDNET_BINARY_TARBALL} (offline install)."
        cp "${BIRDNET_BINARY_TARBALL}" "${workdir}/${archive}"
    else
        info "Downloading ${archive}…"
        if ! download "${archive_url}" "${workdir}/${archive}"; then
            fatal "Archive download failed. Check that release v${version} exists for ${arch}."
        fi

        info "Downloading SHA256SUMS for verification…"
        # A failed SHA256SUMS fetch used to warn and install anyway. That put
        # the choice of whether verification happened in the hands of whoever
        # controlled the network — and anyone able to substitute a 100 MB
        # binary can certainly drop one small request for the file that would
        # expose it. "Could not check" is not a weaker form of "checked out".
        if ! download "${sums_url}" "${workdir}/SHA256SUMS"; then
            fatal "SHA256SUMS could not be downloaded from ${sums_url}, so the archive cannot be verified. Refusing to install an unverified binary. Retry, or fetch the release and its SHA256SUMS by hand, check them with 'sha256sum -c', and install with BIRDNET_BINARY_TARBALL=/path/to/${archive}."
        fi

        # Verify *this* archive, by name.
        #
        # The previous command was `sha256sum -c SHA256SUMS --ignore-missing
        # --status --strict`, which answers a different question: "did anything
        # both listed and present fail?" With GNU coreutils 9.4 that exits 0
        # when our archive was never checked at all, as long as some other
        # listed file was present and matched — so "Checksum verified" could be
        # printed without our archive being verified. Nothing in the current
        # workdir makes that reachable, but the command has to assert what the
        # message claims, not something adjacent that happens to coincide.
        #
        # Narrow to the single line naming this archive first. Any directory
        # prefix and the binary-mode '*' are stripped and the line rewritten
        # with the bare name, because `sha256sum -c` looks up the path exactly
        # as written and would otherwise report a missing file for a
        # `release/${archive}`-style entry.
        local sums_line
        sums_line="$(awk -v want="${archive}" '
            { name = $2; sub(/^\*/, "", name); sub(/^.*\//, "", name)
              if (name == want) printf "%s  %s\n", $1, name }
        ' "${workdir}/SHA256SUMS")"

        if [ -z "${sums_line}" ]; then
            fatal "The published SHA256SUMS for v${version} has no entry for ${archive}, so it cannot be verified. Refusing to install. This usually means the release is incomplete for ${arch}, or the detected architecture is wrong."
        fi
        if [ "$(printf '%s\n' "${sums_line}" | wc -l)" -ne 1 ]; then
            fatal "The published SHA256SUMS for v${version} lists ${archive} more than once. Refusing to guess which digest is authoritative."
        fi

        printf '%s\n' "${sums_line}" >"${workdir}/SHA256SUMS.archive"
        # `--strict` also rejects a malformed digest, so a truncated or
        # HTML-error-page SHA256SUMS fails here rather than appearing to pass.
        if (cd "${workdir}" && sha256sum -c SHA256SUMS.archive --status --strict); then
            success "Checksum verified against SHA256SUMS (${archive})"
        else
            fatal "Checksum mismatch for ${archive} against published SHA256SUMS. The download is corrupt or tampered. Aborting install; nothing was written to ${INSTALL_DIR}."
        fi
    fi

    info "Extracting archive…"
    if ! tar -xzf "${workdir}/${archive}" -C "${workdir}"; then
        fatal "Archive extraction failed. The downloaded file may be corrupt."
    fi

    # The archive contains a single top-level directory named
    # birdnet-behavior-<version>-<target>. Locate the binary inside it.
    local extracted_binary
    # `awk 'NR==1'`, not `head -1`: with more than one match `find` is left
    # writing into a pipe `head` has already closed, and `set -euo pipefail`
    # turns that into a silent exit 141 — the installer stops with no output.
    # Verified: deterministic with 5000 matches, clean with one.
    extracted_binary="$(find "${workdir}" -mindepth 2 -maxdepth 3 -type f -name "${BINARY_NAME}" | awk 'NR==1')"
    if [ -z "${extracted_binary}" ] || [ ! -f "${extracted_binary}" ]; then
        fatal "Could not find '${BINARY_NAME}' binary inside the downloaded archive."
    fi

    # Stop the service here and not a moment earlier. Everything above can
    # fail — an unreachable release, an unverifiable checksum, a corrupt
    # archive — and none of it is a reason to take a working station off the
    # air. From this line on we have a verified binary in hand and the only
    # remaining obstacle is ETXTBSY, which is what the stop is for.
    stop_running_service_for_swap

    install -m 0755 "${extracted_binary}" "${INSTALL_DIR}/${BINARY_NAME}"
    success "Binary installed to ${INSTALL_DIR}/${BINARY_NAME}"

    # Install the bundled operator manual (mdBook) if this release ships it, so
    # the dashboard's /help/* links work fully offline. The service points
    # BNB_HELP_DIR at ${HELP_DIR} (see 65-service.sh). Older releases have no
    # help/ in the tarball — we just skip, and /help 404s as it did before.
    local extracted_help
    extracted_help="$(find "${workdir}" -mindepth 2 -maxdepth 3 -type d -name help | awk 'NR==1')"
    if [ -n "${extracted_help}" ] && [ -d "${extracted_help}" ]; then
        rm -rf "${HELP_DIR}"
        install -d -m 0755 "$(dirname "${HELP_DIR}")"
        cp -a "${extracted_help}" "${HELP_DIR}"
        chmod -R a+rX "${HELP_DIR}"
        success "Operator manual installed to ${HELP_DIR} (served at /help)"
    else
        info "This release has no bundled manual; /help will be unavailable until you upgrade."
    fi
}

# ===== installer/lib/55-model.sh =====
# ---------------------------------------------------------------------------
# Download the BirdNET+ V3.0 model + labels.
#
# Origin: the stable, arch-independent `models-v3.0-preview3` GitHub release
# (so the binary and the model are fetched from the *same* host and the install
# is offline-capable after that one fetch), falling back to Zenodo — the
# upstream source — when the GitHub asset is missing (e.g. installing an older
# app release whose line predates the model release) or unreachable.
#
# Whichever host serves the bytes, each file is verified against the sha256
# pinned in 10-config.sh before it is accepted: those hashes are the integrity
# root of trust (they live in version-controlled, provenance-attested source),
# so a corrupted or tampered download from either origin is rejected and the
# next source is tried. The same hashes are published as the `SHA256SUMS` asset
# of the models release.
# ---------------------------------------------------------------------------

# Verify that FILE has the expected sha256. Returns 0 on a match, 1 on a
# mismatch.
#
# A missing sha256sum is fatal, not a pass. It used to `return 0` — the same
# value a verified file returns — so the one tool that could detect a tampered
# 541 MB model being absent counted as the model being fine. preflight()
# already refuses to run without sha256sum, so this is a backstop rather than
# a path an operator reaches; a backstop that returns success is not one.
verify_model_sha256() {
    local file="$1" expected="$2"
    if ! command -v sha256sum &>/dev/null; then
        # `${file##*/}` rather than basename: the one situation this branch
        # fires in is a broken PATH, and the abort message must not itself
        # depend on an external tool to render.
        fatal "sha256sum is not available, so ${file##*/} cannot be verified. Refusing to install an unverified model. Install coreutils and re-run."
    fi
    local actual
    actual="$(sha256sum "${file}" | awk '{print $1}')"
    if [ "${actual}" = "${expected}" ]; then
        return 0
    fi
    warn "  Checksum mismatch for $(basename "${file}")"
    warn "    expected: ${expected}"
    warn "    actual:   ${actual}"
    return 1
}

# Fetch one model file from the first origin that serves bytes matching its
# pinned sha256. A file is only accepted once the checksum matches; a mismatch
# discards it and falls through to the next origin.
#
#   fetch_verified_model DEST EXPECTED_SHA HUMAN_NAME RESUMABLE LABEL URL [LABEL URL...]
#
# The origins are passed as label/URL pairs rather than derived here, because
# the two model families do not share a URL shape: Zenodo needs
# `<api>/<file>/content` while both GitHub releases take `<base>/<file>`.
# Building them at the call site keeps that knowledge next to the config that
# defines it, and lets the geomodel use its own upstream without teaching this
# function about a third source.
#
# RESUMABLE=1 routes through download_large (resume + progress bar) for the
# ~541 MB classifier; any other value uses the plain download helper.
# Returns 0 once a verified copy is in place, 1 if every origin failed.
fetch_verified_model() {
    local dest="$1" expected_sha="$2" human="$3" resumable="$4"
    shift 4

    while [ "$#" -ge 2 ]; do
        local label="$1" url="$2"
        shift 2

        info "  Fetching ${human} from ${label}…"
        if [ "${resumable}" = "1" ]; then
            if ! download_large "${url}" "${dest}" "${human}"; then
                warn "  ${human}: download from ${label} failed; trying the next source."
                continue
            fi
        else
            if ! download "${url}" "${dest}"; then
                warn "  ${human}: download from ${label} failed; trying the next source."
                continue
            fi
        fi

        if verify_model_sha256 "${dest}" "${expected_sha}"; then
            success "  ${human}: sha256 verified (${label})."
            return 0
        fi

        warn "  ${human}: discarding the file from ${label} and trying the next source."
        rm -f "${dest}"
    done

    return 1
}

# The origin label/URL pairs for one classifier file: our models release first,
# then Zenodo.
classifier_origins() {
    local filename="$1"
    printf '%s\n' \
        "GitHub release ${MODEL_RELEASE_TAG}" "${MODEL_GH_BASE}/${filename}" \
        "Zenodo" "${ZENODO_API}/${filename}/content"
}

# The origin label/URL pairs for one geomodel file: our models release first,
# then the upstream birdnet-team release it is mirrored from.
geomodel_origins() {
    local filename="$1"
    printf '%s\n' \
        "GitHub release ${MODEL_RELEASE_TAG}" "${GEOMODEL_GH_BASE}/${filename}" \
        "upstream birdnet-team/geomodel ${GEOMODEL_VERSION}" "${GEOMODEL_UPSTREAM_BASE}/${filename}"
}

download_model() {
    local model_dest="${MODEL_DIR}/${MODEL_FILE}"
    local labels_dest="${MODEL_DIR}/${LABELS_FILE}"

    # Skip if already present (re-running installer).
    if [ -f "${model_dest}" ] && [ -f "${labels_dest}" ]; then
        success "Model already downloaded at ${MODEL_DIR} — skipping."
        return
    fi

    # Explicit skip: BIRDNET_SKIP_MODEL=1 lets an air-gapped operator stage the
    # ~541 MB model out-of-band (place it at ${MODEL_DIR} later), and lets a CI
    # install smoke test exercise the full flow without the large download. The
    # daemon won't detect until the model is in place, but the install, config,
    # unit, and web UI all come up.
    if [ "${BIRDNET_SKIP_MODEL:-0}" = "1" ]; then
        install -d -m 0750 -o "${SERVICE_USER}" -g "${SERVICE_USER}" "${MODEL_DIR}"
        loud_warn "BIRDNET_SKIP_MODEL=1 — the ML model was NOT downloaded." \
                  "The service will start but will detect NOTHING until you stage it:" \
                  "  place ${MODEL_FILE} and ${LABELS_FILE} in ${MODEL_DIR}," \
                  "  then restart the service."
        return
    fi

    info "Fetching the BirdNET+ V3.0 model (~541 MB FP32 ONNX) + labels…"
    info "  Primary source: GitHub release ${MODEL_RELEASE_TAG} (sha256-verified)."
    info "  Fallback:       Zenodo. This may take a few minutes on a slow link."

    install -d -m 0750 -o "${SERVICE_USER}" -g "${SERVICE_USER}" "${MODEL_DIR}"

    # Model (~541 MB) — resumable so a dropped connection picks up where it left
    # off on the next run instead of restarting from 0 MB.
    if [ ! -f "${model_dest}" ]; then
        local model_origins
        mapfile -t model_origins < <(classifier_origins "${MODEL_FILE}")
        if ! fetch_verified_model "${model_dest}" "${MODEL_SHA256}" \
            "BirdNET+ V3.0 model (~541 MB)" 1 "${model_origins[@]}"; then
            warn "Model download failed or could not be verified from any source."
            warn "Any partial file is kept at:"
            warn "  ${model_dest}"
            warn "Re-run this installer to resume from where it stopped."
            warn "Common causes: no internet connection, GitHub/Zenodo temporarily"
            warn "down, or disk full."
            fatal "Model download failed. Check the cause above and retry."
        fi
        chown "${SERVICE_USER}:${SERVICE_USER}" "${model_dest}"
        success "Model installed to ${model_dest}"
    fi

    # Labels (small file — no resume needed).
    if [ ! -f "${labels_dest}" ]; then
        local labels_origins
        mapfile -t labels_origins < <(classifier_origins "${LABELS_FILE}")
        if ! fetch_verified_model "${labels_dest}" "${LABELS_SHA256}" \
            "species labels CSV" 0 "${labels_origins[@]}"; then
            fatal "Labels download failed or could not be verified. Check your internet connection and retry."
        fi
        chown "${SERVICE_USER}:${SERVICE_USER}" "${labels_dest}"
        success "Labels installed to ${labels_dest}"
    fi
}

# ---------------------------------------------------------------------------
# Download the BirdNET geomodel + its labels (the species occurrence filter).
#
# Deliberately NON-FATAL, unlike the classifier. A station without the
# classifier detects nothing and must stop; a station without the geomodel
# detects everything and merely stops filtering by location, which is exactly
# how every release before this one behaved. Aborting an otherwise good install
# over a 14 MB optional download would be the worse failure — so this warns,
# leaves METADATA_MODEL_PATH unset, and `--doctor` reports the filter as off
# with the command to fix it.
#
# Both files are needed or neither is used: the model's 12 012 outputs are
# meaningless without the label file that names them, and the station refuses a
# model it cannot align. A half-download therefore removes what it got rather
# than leaving a configuration that cannot start.
#
# Sets GEOMODEL_INSTALLED=1 when both files are verified and in place, which is
# what 62-config-file.sh keys the METADATA_* settings on.
# ---------------------------------------------------------------------------
GEOMODEL_INSTALLED=0

download_geomodel() {
    local model_dest="${MODEL_DIR}/${GEOMODEL_FILE}"
    local labels_dest="${MODEL_DIR}/${GEOMODEL_LABELS_FILE}"

    if [ -f "${model_dest}" ] && [ -f "${labels_dest}" ]; then
        GEOMODEL_INSTALLED=1
        success "Geomodel already present at ${MODEL_DIR} — skipping."
        return 0
    fi

    # The same escape hatch the classifier honours: an air-gapped operator
    # stages the files by hand, and the install-smoke CI job skips the fetch.
    if [ "${BIRDNET_SKIP_MODEL:-0}" = "1" ]; then
        info "BIRDNET_SKIP_MODEL=1 — skipping the geomodel download too."
        return 0
    fi

    info "Fetching the BirdNET geomodel ${GEOMODEL_VERSION} (~14 MB) + labels…"
    info "  This is the species occurrence filter: it drops birds that do not"
    info "  occur near this station at this time of year."

    install -d -m 0750 -o "${SERVICE_USER}" -g "${SERVICE_USER}" "${MODEL_DIR}"

    local model_origins labels_origins
    mapfile -t model_origins < <(geomodel_origins "${GEOMODEL_FILE}")
    mapfile -t labels_origins < <(geomodel_origins "${GEOMODEL_LABELS_FILE}")

    if [ ! -f "${model_dest}" ] &&
        ! fetch_verified_model "${model_dest}" "${GEOMODEL_SHA256}" \
            "geomodel (~14 MB)" 0 "${model_origins[@]}"; then
        rm -f "${model_dest}"
        warn "Geomodel download failed or could not be verified from any source."
        warn "The station will run WITHOUT species occurrence filtering: every"
        warn "species the classifier knows stays a candidate wherever it is."
        warn "Re-run this installer to retry, then check with:"
        warn "  birdnet-behavior --doctor"
        return 0
    fi

    if [ ! -f "${labels_dest}" ] &&
        ! fetch_verified_model "${labels_dest}" "${GEOMODEL_LABELS_SHA256}" \
            "geomodel labels" 0 "${labels_origins[@]}"; then
        # The model alone cannot be used, and a configured-but-unusable pair is
        # worse than none: the daemon would refuse it on every start. Remove
        # both so the next run is a clean retry.
        rm -f "${labels_dest}" "${model_dest}"
        warn "Geomodel labels failed to download; removing the model too, since"
        warn "the station cannot use one without the other. Occurrence filtering"
        warn "is OFF. Re-run this installer to retry."
        return 0
    fi

    chown "${SERVICE_USER}:${SERVICE_USER}" "${model_dest}" "${labels_dest}"
    GEOMODEL_INSTALLED=1
    success "Geomodel installed to ${model_dest}"
    success "Species occurrence filtering is ON (threshold SF_THRESH, default 0.03)."
    return 0
}

# ===== installer/lib/60-dirs.sh =====
# ---------------------------------------------------------------------------
# Create data directories and the tmpfs streaming directory
# ---------------------------------------------------------------------------

create_directories() {
    info "Creating data directories…"
    # Directories owned by the service user, not root.
    install -d -m 0750 -o "${SERVICE_USER}" -g "${SERVICE_USER}" \
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
    #
    # NOTE: under systemd the service runs with PrivateTmp=yes, so it gets its
    # OWN /tmp and recreates /tmp/birdnet-stream there on every start (see the
    # ExecStartPre= in install_service). This host-side directory is what a
    # manual `birdnet-behavior --watch-dir /tmp/birdnet-stream` run (outside
    # systemd) uses, and is harmless under the service.
    install -d -m 0750 -o "${SERVICE_USER}" -g "${SERVICE_USER}" "${STREAM_DIR}"

    # If /tmp is NOT already tmpfs, create a dedicated mount.
    if findmnt -t tmpfs /tmp &>/dev/null; then
        success "/tmp is already tmpfs — ${STREAM_DIR} is RAM-backed"
    elif has_systemd; then
        local MOUNT_UNIT="/etc/systemd/system/tmp-birdnet\\x2dstream.mount"
        # 256M leaves headroom over the daemon's rolling raw-segment buffer
        # (STREAM_RETENTION_SECS, ~57 MB/source by default) so a manual, non-
        # systemd run on a non-tmpfs /tmp doesn't hit spurious write failures.
        # tmpfs `size=` is a ceiling, not a reservation — RAM is used only for
        # bytes actually written, which the daemon keeps drained. (Under the
        # systemd service PrivateTmp=yes gives its own /tmp, so this host mount
        # applies to manual runs only.)
        cat > "${MOUNT_UNIT}" <<MEOF
[Unit]
Description=tmpfs for BirdNet-Behavior audio streaming
Before=birdnet-behavior.service

[Mount]
What=tmpfs
Where=${STREAM_DIR}
Type=tmpfs
Options=size=256M,mode=0750,uid=$(id -u "${SERVICE_USER}"),gid=$(id -g "${SERVICE_USER}")

[Install]
WantedBy=multi-user.target
MEOF
        systemctl daemon-reload
        systemctl enable --now "tmp-birdnet\\x2dstream.mount" 2>/dev/null || true
        success "tmpfs mount unit installed for ${STREAM_DIR}"
    else
        # No systemd to manage a tmpfs mount; the plain directory created above
        # is enough for a manual / container run (it just isn't RAM-backed).
        success "Streaming directory ${STREAM_DIR} ready (no systemd tmpfs mount)."
    fi
}

# ===== installer/lib/62-config-file.sh =====
# ---------------------------------------------------------------------------
# Write the default configuration file
# ---------------------------------------------------------------------------

write_config() {
    if [ -f "${CONFIG_FILE}" ]; then
        warn "Config file already exists at ${CONFIG_FILE} — skipping."
        # Upgrade from a version that left the config world-readable: tighten it
        # without touching the user's settings.
        chown "root:${SERVICE_USER}" "${CONFIG_FILE}" 2>/dev/null || true
        chmod 0640 "${CONFIG_FILE}"
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
    local caddy_user_line="# CADDY_USER=admin"
    local caddy_pwd_line="# CADDY_PWD=change-me-to-a-strong-password"
    [ -n "${CADDY_USER_VALUE}" ] && caddy_user_line="CADDY_USER=${CADDY_USER_VALUE}"
    [ -n "${CADDY_PWD_VALUE}" ]  && caddy_pwd_line="CADDY_PWD=${CADDY_PWD_VALUE}"
    # Persist the bind address so `install.sh repair`/`update` keep it (the
    # installer reads BIRDNET_LISTEN back from here on re-run).
    local listen_line="BIRDNET_LISTEN=${LISTEN_ADDR}"
    # The geomodel is optional and its download is non-fatal, so the two
    # settings are only written live when both files actually landed. Writing
    # them unconditionally would point a fresh station at paths that do not
    # exist, and `--doctor` would then report FAIL on an install that had merely
    # declined an optional download.
    local geo_model_line="# METADATA_MODEL_PATH="
    local geo_labels_line="# METADATA_LABELS_PATH="
    if [ "${GEOMODEL_INSTALLED:-0}" = "1" ]; then
        geo_model_line="METADATA_MODEL_PATH=${MODEL_DIR}/${GEOMODEL_FILE}"
        geo_labels_line="METADATA_LABELS_PATH=${MODEL_DIR}/${GEOMODEL_LABELS_FILE}"
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
${alsa_line}
${rtsp_line}
# Multiple RTSP cameras: comma-separated. Each becomes its own capture stream
# (filenames/metrics prefixed RTSP_1, RTSP_2, …). Overrides RTSP_URL when set.
# RTSP_URLS=rtsp://cam1:554/stream,rtsp://cam2:554/stream

# --- Location (used for species frequency filtering and BirdWeather) ---
${lat_line}
${lon_line}
# RECORDING_SCHEDULE=all-day   # all-day | solar | fixed:HH:MM-HH:MM
#                              # solar records sunrise-to-sunset and needs the
#                              # coordinates above. Fixed hours are evaluated in
#                              # UTC, not local time. Also settable in the web UI
#                              # under Settings -> Location & Recording Schedule.

# --- Detection ---
# CONFIDENCE=0.75          # 0.0–1.0, default 0.75 (detections below this are discarded).
#                          # The setup wizard asks for this on first login; note
#                          # that unlike SF_THRESH, 0 does not mean "disabled" —
#                          # it records every window as a detection.
# SENSITIVITY=1.25         # 0.5–1.5, default 1.25 (V2.4 models only; V3.0 ignores it)
# OVERLAP=0.0              # seconds of 3 s analysis window overlap
# DATABASE_LANG=en

# --- Logging ---
# LOG_LEVEL=info           # trace|debug|info|warn|error|off (RUST_LOG wins)
# LOG_MODULES=             # per subsystem, e.g. audio=debug,web=warn
#                          # names: audio detection web db integrations analytics

# --- Species occurrence filtering ---
# Drops species that do not occur near this station at this time of year. Needs
# all three of the following; the installer fetches the last two for you, and
# the two settings below are live when it succeeded, commented out when it did
# not.
#   1. station coordinates (set above, or on the dashboard)
#   2. METADATA_MODEL_PATH   — the BirdNET geomodel (ONNX)
#   3. METADATA_LABELS_PATH  — that model's own label file
# The geomodel scores a different species list from the classifier (12 012
# against 11 560), so the label file is what maps one onto the other; omit it
# only for a metadata model indexed identically to the classifier, which the
# station verifies at startup and refuses if it does not hold.
# Run \`birdnet-behavior --doctor\` to see which of the three is missing.
${geo_model_line}
${geo_labels_line}
# SF_THRESH=0.03           # occurrence threshold; no effect while the filter is off

# --- Disk management ---
# MAX_FILES_SPECIES=0      # 0 = keep all recordings per species; set e.g. 100 to cap
# DISK_PURGE_THRESHOLD=95
# Raw capture segments land in the RAM-backed stream dir (--watch-dir, default
# /tmp/birdnet-stream) and are drained once the detector has processed them, so
# the tmpfs cannot fill. Tune the rolling buffer if needed:
# STREAM_RETENTION_SECS=600  # delete processed raw segments older than this (0 = off)
# STREAM_MAX_MB=512          # hard cap on the stream dir; oldest segments drop first (0 = off)

# --- Notifications (Apprise) ---
# APPRISE_URL=http://localhost:8000

# --- BirdWeather ---
# BIRDWEATHER_TOKEN=your-token-here

# --- Site name shown in web UI ---
# SITENAME=My Bird Station

# --- Web dashboard bind address ---
# Default 0.0.0.0:8502 = reachable from other devices on your LAN. Viewing the
# dashboard is open; the /admin panel (settings, software update) is gated by
# CADDY_PWD below. To restrict the WHOLE dashboard to this device, set
# 127.0.0.1:8502, then apply it with:  sudo bash install.sh repair
${listen_line}

# --- Admin authentication (CADDY_PWD) ---
# The /admin panel can change settings, trigger backups, and update the
# software, so it requires a sign-in through the dashboard's login form.
# A fresh install sets a strong CADDY_PWD automatically; change it here any
# time, then apply it with:  sudo bash install.sh repair
#
# Sign in with the username "admin" — that is the account the dashboard seeds,
# and it is the only one that exists until you create more in the admin panel.
# CADDY_USER below does NOT rename it: the login form reads CADDY_USER from the
# process environment only, and this unit sets no EnvironmentFile, so on a
# bare-metal install it has no effect. It is honoured under Docker, where the
# environment does reach the process.
#
# Clearing CADDY_PWD leaves /admin OPEN to anyone who can reach the dashboard.
${caddy_user_line}
${caddy_pwd_line}
EOF
    # The config can hold secrets (CADDY_PWD, BIRDWEATHER_TOKEN), so keep it
    # readable by the service's group but never world-readable. Root owns it so
    # the non-root service can read but not rewrite its own config.
    chown "root:${SERVICE_USER}" "${CONFIG_FILE}"
    chmod 0640 "${CONFIG_FILE}"
    success "Default config written — edit ${CONFIG_FILE} to configure your station."
}

# ===== installer/lib/65-service.sh =====
# ---------------------------------------------------------------------------
# Install the systemd service unit
# ---------------------------------------------------------------------------

# Decide the dashboard bind address, PRESERVING it across re-runs so a repair or
# update never silently re-hides a LAN-exposed dashboard back on localhost.
# Precedence (highest first):
#   1. BIRDNET_LISTEN in the environment (explicit override; already in LISTEN_ADDR)
#   2. BIRDNET_LISTEN= in the config file (the operator-editable source of truth)
#   3. --listen in an existing service unit (carry the previous choice forward)
#   4. the interactive prompt / default already in LISTEN_ADDR (fresh installs)
resolve_listen_addr() {
    [ -n "${BIRDNET_LISTEN:-}" ] && return 0

    # The `|| true` keeps a no-match grep (exit 1) from tripping `set -o pipefail`
    # + `set -e` and aborting the whole installer — the common case is a config
    # with no uncommented BIRDNET_LISTEN.
    local from_cfg=""
    if [ -f "${CONFIG_FILE}" ]; then
        from_cfg="$(grep -E '^[[:space:]]*BIRDNET_LISTEN[[:space:]]*=' "${CONFIG_FILE}" 2>/dev/null \
            | tail -1 | cut -d= -f2- | tr -d '[:space:]' || true)"
    fi
    if [ -n "${from_cfg}" ]; then
        LISTEN_ADDR="${from_cfg}"
        info "Dashboard bind address from config: ${LISTEN_ADDR}"
        return 0
    fi

    if [ -f "${SERVICE_FILE}" ]; then
        local from_unit
        from_unit="$(grep -oE -- '--listen [^ ]+' "${SERVICE_FILE}" 2>/dev/null \
            | awk 'NR==1 {print $2}' || true)"
        if [ -n "${from_unit}" ] && [ "${from_unit}" != "${LISTEN_ADDR}" ]; then
            LISTEN_ADDR="${from_unit}"
            info "Preserving dashboard bind address from the existing unit: ${LISTEN_ADDR}"
        fi
    fi
}

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
# Wait for the filesystem holding the database, recordings, and model. A no-op
# when the data dir is on the root filesystem (the usual case); load-bearing
# when it is an external USB disk, where systemd would otherwise start the
# service against an empty mount point and the doctor preflight below would
# fail on an unwritable recordings directory.
RequiresMountsFor=${DATA_DIR}
# No permanent give-up. This used to be StartLimitBurst=5 /
# StartLimitIntervalSec=300, which marks the unit failed after five restarts in
# five minutes and then stops trying — "for operator review (visible in the web
# UI's health page once the service comes back)", which is circular: the web UI
# *is* this service. An unattended box in a field would stay down until someone
# walked to it.
#
# Five restarts at RestartSec=10 is under a minute, so any cause that clears in
# a minute or two hit it: an external data disk that mounts late, a port still
# held by the previous process, a card that needs a second read. All recover on
# their own; none of them recovered from a unit systemd had given up on.
#
# So the rate limit is off and the tight-loop concern — which was real — is
# handled by backing off instead: 10 s, then longer, up to 5 minutes between
# attempts. A permanently broken install retries quietly every five minutes
# forever (the journal rate limits below cap the noise) and comes back by
# itself the moment its cause is fixed.
StartLimitIntervalSec=0

[Service]
# Type=notify pairs with sd_notify in src/sd_notify.rs:
#   - READY=1 when the web server has bound its socket
#   - WATCHDOG=1 periodic pings keep the watchdog happy
#   - STOPPING=1 on graceful shutdown
Type=notify
NotifyAccess=main
User=${SERVICE_USER}

# Serve the bundled operator manual (mdBook) at /help/*. install.sh installs it
# from the release tarball to ${HELP_DIR}; harmless if absent on older releases —
# the ServeDir simply returns 404 for /help, exactly as before.
Environment=BNB_HELP_DIR=${HELP_DIR}

# Recreate the ephemeral stream/watch dir before anything else runs. With
# PrivateTmp=yes (below) the service gets a FRESH, EMPTY /tmp on every start,
# so /tmp/birdnet-stream never survives a restart — create it here so the
# file-watcher has somewhere to attach and the doctor preflight sees a
# writable recordings dir. The binary also creates it at startup; doing it
# here keeps older binaries working too.
#
# IMPORTANT: ${STREAM_DIR} must NOT appear in ReadWritePaths= below. PrivateTmp
# mounts a new tmpfs over /tmp, and bind-mounting a path *beneath* that new
# mount fails namespace setup with "${STREAM_DIR}: No such file or directory"
# (which is exactly the start failure this avoids). The private /tmp is already
# writable, so the watch dir does not need to be in ReadWritePaths.
ExecStartPre=/bin/mkdir -p ${STREAM_DIR}

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
# Exponential backoff between 10 s and 5 minutes (systemd >= 254; older
# versions log "Unknown key name" and fall back to a constant RestartSec=10,
# which is the previous behaviour, so this is safe to ship everywhere).
RestartSteps=10
RestartMaxDelaySec=300
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

# Resource ceilings — cap a runaway process without starving the workload.
# The bundled DuckDB analytics engine is on by default and its queries can be
# memory-hungry under load; 1 GiB leaves that headroom (the FP32 model is
# mmap'd, so its pages are reclaimable and don't count as anonymous RSS). On a
# multi-GB Pi this is the binding limit; on a 512 MB board physical RAM + zram
# bind first, so raising the cgroup ceiling here is harmless there.
MemoryHigh=768M
MemoryMax=1G
TasksMax=512
LimitNOFILE=65536
LimitNPROC=256
# Recover gracefully under memory pressure.
OOMScoreAdjust=200
OOMPolicy=stop

# ── Filesystem isolation ─────────────────────────────────────────────────
# Read-only access to the rest of the filesystem; explicit write paths.
# ${STREAM_DIR} is intentionally absent — it lives in the PrivateTmp /tmp
# (see ExecStartPre= above); listing it here would break namespace setup.
ProtectSystem=strict
ProtectHome=read-only
ReadWritePaths=${DATA_DIR} ${CONFIG_DIR} /run /var/log
PrivateTmp=yes
ProtectKernelLogs=yes
ProtectKernelModules=yes
ProtectKernelTunables=yes
ProtectControlGroups=yes
ProtectClock=yes
ProtectHostname=yes
ProtectProc=invisible
# Deliberately NOT ProcSubset=pid. The Station Health "Vitals" read the
# system-wide /proc files — /proc/stat and /proc/cpuinfo (CPU %, core count)
# and /proc/meminfo (memory), via the sysinfo crate, plus /proc/uptime.
# ProcSubset=pid hides exactly those non-process files, which made the
# dashboard report 0 CPU cores and 0 B memory while temperature/disk (read
# from /sys and statvfs) still worked. Leave /proc at the default (all);
# ProtectProc=invisible above still hides other users' processes.
# Block-listed kernel surfaces; we don't need them.
RestrictSUIDSGID=yes
RestrictRealtime=yes
RestrictNamespaces=yes
LockPersonality=yes
MemoryDenyWriteExecute=yes
NoNewPrivileges=yes
# Drop every capability from the bounding set — a non-root network + audio
# service needs none, and with NoNewPrivileges it can never regain them.
CapabilityBoundingSet=
# Files the service creates (database, recordings) are group-readable at most,
# never world-readable.
UMask=0027
# Restrict sockets to what a web service + journald + local-IP lookup require.
RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6 AF_NETLINK
SystemCallArchitectures=native
# Permit only POSIX, file I/O, networking, and signals — explicitly
# excludes things like raw_io / module_load / ptrace / mount / reboot.
SystemCallFilter=@system-service
SystemCallFilter=~@privileged @resources @mount @debug @cpu-emulation @obsolete @reboot @swap @raw-io @clock @module

# Audio access — must keep these capability sets / device mounts.
#
# DeviceAllow= resolves a path to a *device node*. /dev/snd is a DIRECTORY, so
# "DeviceAllow=/dev/snd rw" matches nothing: with DevicePolicy=closed every ALSA
# node (/dev/snd/pcmC1D0c, controlC1, …) stayed denied, and microphone capture
# could never work on a bare-metal install. arecord still exec'd successfully —
# so the daemon logged "started microphone capture" — and only then failed the
# PCM open with "audio open error: No such file or directory", which the
# supervisor saw as a stalled source and restarted forever.
#
# char-alsa is systemd's documented subsystem form and is what actually grants
# the nodes. Verified on a Raspberry Pi 4 (Pi OS Trixie, USB mic on card 1) with
# an A/B under systemd-run: "/dev/snd rw" fails to open the device, "char-alsa
# rw" records normally. RTSP stations were unaffected — ffmpeg never touches
# /dev/snd — which is why this survived from v0.6.0 to v0.11.0 unnoticed.
SupplementaryGroups=audio
DeviceAllow=char-alsa rw
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

    if has_systemd; then
        systemctl daemon-reload
        systemctl enable birdnet-behavior.service
        success "Service installed and enabled (Type=notify, hardened, watchdog active)."
    else
        success "Service unit written to ${SERVICE_FILE} (Type=notify, hardened, watchdog active)."
        warn "systemd is not running here — not enabling/starting the unit."
        warn "  On a systemd host, finish with:"
        warn "    sudo systemctl daemon-reload && sudo systemctl enable --now birdnet-behavior"
    fi
}

# ===== installer/lib/70-station.sh =====
# ---------------------------------------------------------------------------
# Detect and configure the audio device + station location
# ---------------------------------------------------------------------------

# Returns the first detected ALSA capture device as "plughw:<card>,<device>",
# or an empty string if none found / arecord not available.
detect_first_audio_device() {
    # No arecord means no card list AND no capture backend. check_required_tools
    # tries to install alsa-utils before we get here, so reaching this branch
    # means that failed (or apt-get is not this distro's package manager) —
    # say so, because a silent empty result reads as "no microphone attached"
    # and produces a station that records nothing without ever complaining.
    if ! command -v arecord &>/dev/null; then
        warn "arecord not found — cannot detect a microphone. Install alsa-utils, then:"
        warn "  sudo bash install.sh repair"
        return 0
    fi
    # arecord -l output looks like:
    #   card 1: PRO [Comica_Traxshot PRO], device 0: USB Audio [USB Audio]
    #          ^index ^ALSA card id
    local listing first_card first_id first_device
    listing="$(arecord -l 2>/dev/null)"
    [ -n "${listing}" ] || return 0

    first_card="$(printf '%s\n' "${listing}" | awk '/^card /{ print $2; exit }' | tr -d ':')"
    [ -n "${first_card}" ] || return 0
    first_id="$(printf '%s\n' "${listing}" | awk '/^card /{ print $3; exit }')"
    # 2-arg match() (RSTART/RLENGTH) is POSIX; the 3-arg capture form is a gawk
    # extension that errors on mawk (the default awk on Debian / Raspberry Pi OS).
    first_device="$(printf '%s\n' "${listing}" \
        | awk '/^card /{ if (match($0, /device [0-9]+/)) print substr($0, RSTART + 7, RLENGTH - 7); exit }')"

    # Prefer the card's ALSA *id* over its *index*.
    #
    # A card index is assigned in detection order and is not stable: it changes
    # when USB devices are re-enumerated, which a reboot is free to do. Measured
    # on a Raspberry Pi 4 during the acceptance run — the same microphone was
    # `card 1: PRO` before a cold reboot and `card 3: PRO` after it. The index
    # moved; the id did not. A station configured with the index came back from
    # that reboot serving a healthy dashboard and recording nothing, retrying a
    # device that no longer existed, forever.
    #
    # `plughw:CARD=<id>,DEV=<n>` addresses the card by that id. alsa-lib's own
    # alsa.conf declares `pcm.plughw { @args [ CARD DEV SUBDEV ] }` with
    # `@args.CARD { type string }`, forwarded to a `type hw` slave as
    # `card $CARD`, so a name is a first-class argument rather than a trick.
    #
    # This is the same identity `usb-audio-mapper` pins via a udev rule
    # (`ATTR{id}="<friendly_name>"`), so a station set up with that tool gets a
    # name the operator chose, and one that survives identical devices being
    # swapped between ports. See docs/book/admin/audio.md.
    #
    # Fall back to the index when the id cannot be trusted to identify one card:
    # two cards sharing an id would make `CARD=` ambiguous, and a non-portable
    # id would need quoting we cannot guarantee downstream.
    local id_count=0
    if [ -n "${first_id}" ]; then
        id_count="$(printf '%s\n' "${listing}" \
            | awk -v id="${first_id}" '$1 == "card" && $3 == id { n++ } END { print n+0 }')"
    fi
    if [ -n "${first_id}" ] \
        && grep -qE '^[A-Za-z0-9_-]+$' <<<"${first_id}" \
        && [ "${id_count}" = "1" ]; then
        echo "plughw:CARD=${first_id},DEV=${first_device:-0}"
    else
        echo "plughw:${first_card},${first_device:-0}"
    fi
}

# True (0) if v is a decimal number within [lo, hi]. Used to sanity-check
# latitude/longitude so a typo doesn't get written into the config.
valid_coord() {
    awk -v v="$1" -v lo="$2" -v hi="$3" \
        'BEGIN { if (v ~ /^[-+]?([0-9]+\.?[0-9]*|\.[0-9]+)$/ && v+0 >= lo && v+0 <= hi) exit 0; exit 1 }'
}

# Read whatever the operator typed at the coordinate prompt and print either
# "lat lon" or a bare "lat", or nothing at all if it is not coordinates.
#
# The prompt used to accept exactly one shape — a lone dotted decimal — which
# was at odds with the tip printed directly above it: right-clicking on
# OpenStreetMap hands you a *pair*, "49.4521, 8.6724", and pasting that was
# rejected. It was also at odds with the rest of the product, since the web
# settings form accepts a decimal comma. Both now parse:
#
#   49.4521             a lone latitude; the longitude is asked for next
#   49.4521, 8.6724     the pair OpenStreetMap gives you
#   49,4521             decimal comma
#   49,4521 8,6724      decimal commas, space separated
#   49,4521,8,6724      decimal commas, comma separated
#
# The one real ambiguity is a single comma: is "49,45" the decimal 49.45 or the
# pair (49, 45)? It resolves on range wherever it can — in "49,4521" the tail
# 4521 is not a valid longitude, so the comma has to be a decimal point. Where
# both readings are valid the pair wins, matching the OpenStreetMap flow the
# tip sends people to; the caller echoes the result back so a wrong guess is
# visible and correctable rather than silently written to the config.
parse_coords() {
    awk -v s="$1" '
        function strip(x) { sub(/,$/, "", x); return x }
        function norm(x)  { gsub(/,/, ".", x); return x }
        function ok(x, lo, hi) {
            return x ~ /^[-+]?([0-9]+\.?[0-9]*|\.[0-9]+)$/ && x + 0 >= lo && x + 0 <= hi
        }
        BEGIN {
            gsub(/^[ \t]+|[ \t]+$/, "", s)
            n = split(s, tok, /[ \t]+/)

            if (n == 2) {
                a = norm(strip(tok[1])); b = norm(tok[2])
                if (ok(a, -90, 90) && ok(b, -180, 180)) print a " " b
                exit
            }
            if (n != 1) exit

            t = tok[1]
            if (index(t, ",") == 0) {            # a plain dotted decimal
                if (ok(t, -90, 90)) print t
                exit
            }
            m = split(t, p, ",")
            if (m == 2) {
                if (ok(p[1], -90, 90) && ok(p[2], -180, 180)) { print p[1] " " p[2]; exit }
                c = p[1] "." p[2]                # so the comma was a decimal point
                if (ok(c, -90, 90)) print c
                exit
            }
            if (m == 4) {                        # "49,4521,8,6724"
                a = p[1] "." p[2]; b = p[3] "." p[4]
                if (ok(a, -90, 90) && ok(b, -180, 180)) print a " " b
            }
        }'
}

# Generate a strong, shell/URL-friendly random password.
#
# Neither branch may end in a consumer that stops reading early. The fallback
# used to be `tr -dc … </dev/urandom | head -c 22`: /dev/urandom never ends, so
# `head` always exits with the producer mid-write, `tr` always takes SIGPIPE,
# and under this script's `set -o pipefail` the pipeline always returns 141.
# The caller assigns the result (`CADDY_PWD_VALUE="$(gen_password)"`), so under
# `set -e` that killed the installer outright, silently, at the exact step that
# secures /admin. Measured: 200 failures in 200 runs.
#
# Only systems without openssl reached it, which is why it survived — Raspberry
# Pi OS and Debian both ship openssl, and that branch ends in `cut`, which reads
# its input to the end.
#
# So the randomness is now bounded at the source and consumed whole.
gen_password() {
    local raw=""
    if command -v openssl &>/dev/null; then
        raw="$(openssl rand -base64 48 2>/dev/null | tr -dc 'A-Za-z0-9')"
    fi
    if [ "${#raw}" -lt 22 ]; then
        # `head -c` on a *file* is a bounded read, not a pipeline: nothing is
        # left writing, so there is no SIGPIPE to take. 4096 random bytes yield
        # ~990 alphanumerics, so 22 is never short.
        raw="$(head -c 4096 /dev/urandom 2>/dev/null | LC_ALL=C tr -dc 'A-Za-z0-9')"
    fi
    if [ "${#raw}" -lt 22 ]; then
        fatal "Could not generate a random admin password (no openssl, and /dev/urandom yielded ${#raw} usable characters). Set CADDY_PWD in ${CONFIG_FILE} by hand."
    fi
    printf '%s' "${raw:0:22}"
}

# Guarantee the /admin panel is password-protected on a fresh LAN install. The
# dashboard binds to the LAN by default and viewing is open, but admin actions
# (settings, software update) must require a password — so if the operator
# didn't set one during onboarding, generate a strong one. No-ops when:
#   - the config already exists (never touch an operator's existing credentials)
#   - a password was already chosen during onboarding
#   - the dashboard is bound to localhost only (admin exposure is local anyway)
ensure_admin_password() {
    [ -f "${CONFIG_FILE}" ] && return 0
    [ -n "${CADDY_PWD_VALUE}" ] && return 0
    case "${LISTEN_ADDR}" in 127.0.0.1:* | localhost:*) return 0 ;; esac

    CADDY_USER_VALUE="admin"
    CADDY_PWD_VALUE="$(gen_password)"
    GENERATED_ADMIN_PASSWORD="${CADDY_PWD_VALUE}"
    info "Generated an admin password for the dashboard (shown at the end)."
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
        while :; do
            audio_in="$(ask "  Audio source — ALSA device (e.g. plughw:1,0) or rtsp:// URL (Enter to skip)" "")"
            case "${audio_in}" in
                '')
                    break ;;
                rtsp://?* | rtsps://?*)
                    RTSP_URL_VALUE="${audio_in}"; break ;;
                *://*)
                    # URL-like input whose scheme isn't rtsp(s):// is almost
                    # certainly a typo (e.g. http://…). Reject and re-prompt
                    # rather than silently storing it as an ALSA device string
                    # (which the capture path could never open). ALSA devices
                    # (plughw:1,0, hw:0, default) contain no '://', so this only
                    # catches mistyped stream URLs.
                    warn "  A stream URL must start with rtsp:// or rtsps:// — got '${audio_in}'. Try again." ;;
                *)
                    ALSA_CARD_VALUE="${audio_in}"; break ;;
            esac
        done
    fi
    if [ -n "${ALSA_CARD_VALUE}" ]; then
        success "Audio source: ALSA ${ALSA_CARD_VALUE}"
    elif [ -n "${RTSP_URL_VALUE}" ]; then
        success "Audio source: RTSP ${RTSP_URL_VALUE}"
    else
        warn "No audio source set — add ALSA_CARD or RTSP_URL to ${CONFIG_FILE} later."
    fi

    # ---- Station location ----
    #
    # This loops, like the audio-source prompt above it. It used to warn once
    # and fall through, which meant unreadable input was discarded into a
    # single [WARN] line that scrolled away behind the 541 MB model download —
    # the operator answered the question, the station recorded no location, and
    # nothing said so again until the species filter silently stayed off.
    printf '\n  Station location (solar schedule, species filter, BirdWeather)\n' >/dev/tty
    printf '  Tip: right-click your spot on https://openstreetmap.org and read off the coordinates.\n' >/dev/tty
    printf '  Paste both at once (49.4521, 8.6724) or enter them one at a time.\n' >/dev/tty
    printf '  A decimal comma (49,4521) is fine.\n' >/dev/tty
    local coord_in lon_in coords
    while :; do
        coord_in="$(ask "  Latitude — or both coordinates (Enter to skip)" "")"
        if [ -z "${coord_in}" ]; then
            warn "No location set — species filtering stays off until you set one."
            warn "  Add LATITUDE/LONGITUDE to ${CONFIG_FILE}, or use the dashboard's setup wizard."
            break
        fi
        coords="$(parse_coords "${coord_in}")"
        if [ -z "${coords}" ]; then
            warn "  Could not read '${coord_in}' as coordinates — try again, or press Enter to skip."
            continue
        fi
        # A lone latitude: collect its other half, then re-parse the two
        # together so the pair goes through exactly one validation path.
        case "${coords}" in
            *' '*) : ;;
            *)
                lon_in="$(ask "  Longitude (e.g. 8.6724)" "")"
                coords="$(parse_coords "${coords} ${lon_in}")"
                ;;
        esac
        case "${coords}" in
            *' '*)
                LATITUDE_VALUE="${coords%% *}"
                LONGITUDE_VALUE="${coords##* }"
                success "Location: ${LATITUDE_VALUE}, ${LONGITUDE_VALUE}"
                break
                ;;
            *)
                warn "  A latitude on its own does not locate the station — try again, or press Enter to skip."
                ;;
        esac
    done

    # ---- Web dashboard ----
    printf '\n  Web dashboard\n' >/dev/tty
    printf '  The dashboard is reachable from other devices on your network. Viewing is\n' >/dev/tty
    printf '  open; the admin panel (settings, software update) is protected by a password.\n' >/dev/tty
    local pw1 pw2
    pw1="$(ask_secret "  Set an admin password now (Enter to auto-generate one)")"
    if [ -n "${pw1}" ]; then
        pw2="$(ask_secret "  Confirm password")"
        if [ "${pw1}" = "${pw2}" ]; then
            CADDY_USER_VALUE="admin"
            CADDY_PWD_VALUE="${pw1}"
            success "Admin password set (username: admin)."
        else
            warn "Passwords did not match — a strong one will be generated instead."
        fi
    fi
    # The dashboard intentionally binds all interfaces (0.0.0.0:8502) so it is
    # reachable from a phone or laptop out of the box. Restricting it to this
    # host is an advanced, easy-to-misfire choice — it strands a non-technical
    # operator who then "can't open the page" — so it is deliberately NOT a
    # setup-wizard question. Advanced users opt in explicitly by setting
    # BIRDNET_LISTEN=127.0.0.1:8502 in the environment or the config file, which
    # resolve_listen_addr honours (and the installer preserves across re-runs).
}

# ===== installer/lib/75-start.sh =====
# ---------------------------------------------------------------------------
# Start the service when appropriate
# ---------------------------------------------------------------------------

# True (0) if the config has an *active* (uncommented) audio source — ALSA_CARD,
# RTSP_URL(S), or PIPEWIRE_DEVICE. Used only to tailor the post-install message;
# the service is started regardless (a station with no source still serves the
# dashboard + onboarding wizard, exactly as it does on a reboot).
config_has_audio_source() {
    grep -qE '^[[:space:]]*(ALSA_CARD|RTSP_URL|RTSP_URLS|PIPEWIRE_DEVICE)[[:space:]]*=' \
        "${CONFIG_FILE}" 2>/dev/null
}

# True (0) if the config has *both* an active LATITUDE and LONGITUDE.
#
# Same "tailor the message" role as config_has_audio_source, and the same
# reason for existing: without coordinates the metadata model cannot run, so
# occurrence filtering is skipped and every species in the model stays a
# candidate. The station looks like it is working — it just reports birds from
# the wrong continent, which reads as a bad model rather than a missing
# setting. A non-interactive install (`BIRDNET_NONINTERACTIVE=1`, or no TTY
# under `curl | sudo bash`) never reaches the location prompt at all, so this
# is the common state, not the rare one.
#
# Both halves are required: `birdnet_core::config::validate` already warns
# about one without the other, and one alone disables the filter just as
# completely as neither.
config_has_location() {
    grep -qE '^[[:space:]]*LATITUDE[[:space:]]*=[[:space:]]*[^[:space:]]' \
        "${CONFIG_FILE}" 2>/dev/null \
    && grep -qE '^[[:space:]]*LONGITUDE[[:space:]]*=[[:space:]]*[^[:space:]]' \
        "${CONFIG_FILE}" 2>/dev/null
}

maybe_start_service() {
    # No systemd here (container / chroot / staged install): the unit is on disk
    # but there is nothing to start it. install_service already told the operator
    # how to finish on a real host; nothing more to do.
    if ! has_systemd; then
        return
    fi

    # Upgrade path: if we stopped a running service to swap the binary, bring
    # it back on the new version. Schema migrations run automatically on
    # startup, and the SQLite/DuckDB data + config were left untouched.
    if [ "${SERVICE_WAS_RUNNING}" = "1" ]; then
        info "Restarting service on the upgraded binary…"
        # Cleared first: restore_service_if_we_stopped_it is armed as an EXIT
        # trap and must not start it a second time.
        SERVICE_WAS_RUNNING=0
        systemctl start birdnet-behavior.service
        success "Service restarted (schema migrations applied on startup)."
        return
    fi

    # Fresh install: start the service now so the dashboard comes up immediately.
    # The unit is enabled (see install_service), so systemd brings it up on the
    # next reboot no matter what — starting it here closes the confusing gap
    # where nothing appears after install but a reboot "fixes" it.
    #
    # An audio source is deliberately NOT required to start. The web dashboard —
    # and its first-run onboarding wizard, where the operator picks a microphone
    # and sets their location — is the whole point of a fresh install, and the
    # detection daemon idles harmlessly until a source exists. The unit's doctor
    # preflight treats "no audio source" as a warning, not a failure, so the
    # start succeeds either way. Mirrors the Docker quickstart, which has always
    # brought the dashboard up regardless of audio.
    info "Starting service now…"
    if systemctl start birdnet-behavior.service; then
        if config_has_audio_source; then
            success "Service started."
        else
            success "Service started — finish setup in the dashboard."
            info  "No audio source yet: pick a microphone in the dashboard's setup wizard"
            info  "(or set ALSA_CARD / RTSP_URL in ${CONFIG_FILE}); detection begins once one is set."
        fi
    else
        warn "Service failed to start — inspect: sudo journalctl -u birdnet-behavior -e"
        warn "Once resolved: sudo systemctl start birdnet-behavior"
    fi
}

# ===== installer/lib/76-validate.sh =====
# ---------------------------------------------------------------------------
# Post-install validation
#
# After an install / update / repair, confirm the result is actually healthy
# and report each check. Advisory by design: it never aborts (the install
# already happened), but a FAIL line tells the operator exactly what to fix.
# ---------------------------------------------------------------------------

# Run a command as the service user so writability/ownership checks reflect
# what the daemon will actually see (root can write everywhere; the service
# user cannot). Falls back gracefully when runuser/sudo are unavailable.
run_as_service_user() {
    if command -v runuser &>/dev/null; then
        runuser -u "${SERVICE_USER}" -- "$@"
    elif command -v sudo &>/dev/null; then
        sudo -n -u "${SERVICE_USER}" -- "$@"
    else
        "$@"
    fi
}

# Number of validation problems found, for the caller's summary line.
VALIDATION_FAILURES=0

_v_pass() { success "  check: $*"; }
_v_warn() { warn "  check: $*"; }
_v_fail() { error "  check: $*"; VALIDATION_FAILURES=$((VALIDATION_FAILURES + 1)); }

validate_install() {
    info "Validating installation…"
    VALIDATION_FAILURES=0

    # 1. Binary runs.
    if [ -x "${INSTALL_DIR}/${BINARY_NAME}" ] \
        && "${INSTALL_DIR}/${BINARY_NAME}" --version &>/dev/null; then
        _v_pass "binary executes ($("${INSTALL_DIR}/${BINARY_NAME}" --version 2>/dev/null | awk 'NR==1'))"
    else
        _v_fail "binary at ${INSTALL_DIR}/${BINARY_NAME} is missing or won't run"
    fi

    # 2. Service unit parses. systemd-analyze verify catches typos and a number
    #    of sandboxing mistakes before they cause a start failure.
    if [ -f "${SERVICE_FILE}" ]; then
        if command -v systemd-analyze &>/dev/null; then
            local verify_out
            if verify_out="$(systemd-analyze verify "${SERVICE_FILE}" 2>&1)"; then
                _v_pass "systemd unit verifies clean"
            else
                _v_warn "systemd-analyze verify reported: ${verify_out}"
            fi
        else
            _v_pass "service unit present (systemd-analyze not available to verify)"
        fi
    else
        _v_fail "service unit ${SERVICE_FILE} is missing"
    fi

    # 3. Data directories exist and belong to the service user.
    local d owner
    for d in "${DATA_DIR}" "${RECS_DIR}" "${MODEL_DIR}"; do
        if [ ! -d "${d}" ]; then
            _v_fail "directory missing: ${d}"
            continue
        fi
        owner="$(stat -c '%U' "${d}" 2>/dev/null || echo '?')"
        if [ "${owner}" = "${SERVICE_USER}" ]; then
            _v_pass "${d} owned by ${SERVICE_USER}"
        else
            _v_fail "${d} owned by ${owner}, expected ${SERVICE_USER} (run: install.sh repair)"
        fi
    done

    # 4. Config is readable by the daemon (service user, via group).
    if [ -f "${CONFIG_FILE}" ]; then
        if run_as_service_user test -r "${CONFIG_FILE}" 2>/dev/null; then
            _v_pass "config readable by ${SERVICE_USER}"
        else
            _v_fail "${CONFIG_FILE} not readable by ${SERVICE_USER} (run: install.sh repair)"
        fi
    fi

    # 5. Doctor preflight, run as the service user (mirrors ExecStartPre).
    local rc=0
    run_as_service_user "${INSTALL_DIR}/${BINARY_NAME}" --doctor --config "${CONFIG_FILE}" \
        &>/dev/null || rc=$?
    case "${rc}" in
        0) _v_pass "doctor preflight passed" ;;
        1) _v_warn "doctor preflight passed with warnings (run: ${BINARY_NAME} --doctor --config ${CONFIG_FILE})" ;;
        *) _v_fail "doctor preflight reported errors (run: ${BINARY_NAME} --doctor --config ${CONFIG_FILE})" ;;
    esac

    # 6. If the service is up, confirm the web port is actually listening.
    if systemctl is-active --quiet "${SERVICE_NAME}" 2>/dev/null; then
        local port="${LISTEN_ADDR##*:}"
        # Capture first, then match against a here-string. `producer | grep -q`
        # is a trap under `set -o pipefail`: grep exits on its first match, the
        # producer takes SIGPIPE, and the pipeline reports 141 — so a match
        # reads as a miss. Measured at 1 in 300 with a 5000-line producer. Here
        # that meant reporting "port not seen listening" about a port that was.
        local listening=""
        if command -v ss &>/dev/null; then
            listening="$(ss -ltn 2>/dev/null || true)"
        fi
        if grep -q ":${port}\b" <<<"${listening}"; then
            _v_pass "service active and listening on port ${port}"
        else
            _v_warn "service active but port ${port} not seen listening yet (it may still be starting)"
        fi
    else
        info "  check: service not running yet (start it once an audio source is set)."
    fi

    if [ "${VALIDATION_FAILURES}" -eq 0 ]; then
        success "Validation passed."
    else
        warn "Validation found ${VALIDATION_FAILURES} problem(s) — see the check lines above."
    fi
}

# ===== installer/lib/77-manage.sh =====
# ---------------------------------------------------------------------------
# Top-level flows: install / update / reinstall / repair / uninstall, and the
# interactive menu shown when an existing install is detected.
# ---------------------------------------------------------------------------

# Put a service we stopped back, if the run is ending without having restarted
# it. Installed as an EXIT trap by stop_running_service_for_swap.
#
# Every `fatal` between the stop and maybe_start_service used to leave a working
# station switched off: a failed model download, an unwritable directory, and —
# since verification became mandatory — an unreachable SHA256SUMS. An update
# that cannot proceed must leave the station exactly as it found it, running.
restore_service_if_we_stopped_it() {
    local rc=$?
    if [ "${SERVICE_WAS_RUNNING:-0}" = "1" ] && has_systemd; then
        if [ "${rc}" -ne 0 ]; then
            warn "The run is ending unsuccessfully; restarting the service that was stopped for the swap."
        fi
        systemctl start "${SERVICE_NAME}" 2>/dev/null \
            || warn "Could not restart ${SERVICE_NAME} — start it with: sudo systemctl start birdnet-behavior"
        SERVICE_WAS_RUNNING=0
    fi
    return "${rc}"
}

# Stop a running service before swapping the binary. You cannot overwrite a
# running executable in place (ETXTBSY), and a plain `systemctl start` on an
# already-running unit would not load the new binary. Records that it was
# running so the service is restarted afterwards — and arms the EXIT trap so
# that happens even when the run does not reach maybe_start_service.
stop_running_service_for_swap() {
    has_systemd || return 0
    if systemctl is-active --quiet "${SERVICE_NAME}" 2>/dev/null; then
        SERVICE_WAS_RUNNING=1
        trap restore_service_if_we_stopped_it EXIT
        info "Stopping the running service to swap the binary safely…"
        systemctl stop "${SERVICE_NAME}" || true
    fi
}

# Offer ZRAM compressed swap on boards with <= 2 GB RAM (Pi Zero 2W, Pi 2,
# etc.). Silently skipped on machines with adequate RAM or where it is off.
maybe_setup_zram() {
    local mem_mb
    mem_mb="$(awk '/MemTotal/ {printf "%d", $2/1024}' /proc/meminfo 2>/dev/null || echo 9999)"
    if [ "${mem_mb}" -le 2048 ] && [ "${SKIP_ZRAM:-0}" != "1" ]; then
        info "Low-RAM system detected (${mem_mb} MB) — setting up ZRAM compressed swap…"
        setup_zram || warn "ZRAM setup failed (non-fatal); continuing without it."
    fi
}

# The full, idempotent install flow. Used for a fresh install and — because
# every step is safe to repeat — for `update` and `reinstall` as well.
do_install() {
    local arch version
    arch="$(detect_arch)"
    check_glibc
    check_required_tools
    check_disk_space
    version="$(resolve_version)"

    info "Arch: ${arch}, Version: ${version}"

    # NOTE: the running service is *not* stopped here. install_binary stops it
    # itself, immediately before the swap, so a download or checksum failure
    # never takes a working station off the air for a binary that was never
    # installed.
    install_binary "${version}" "${arch}"
    create_directories
    setup_tmpfs_streaming
    download_model
    download_geomodel        # optional: the species occurrence filter
    prompt_station_settings
    resolve_listen_addr      # finalize the bind address (env > config > unit > prompt)
    ensure_admin_password    # auto-protect /admin on a fresh LAN install
    write_config             # bakes BIRDNET_LISTEN + CADDY_* into the config
    ensure_capture_backend
    install_service
    maybe_setup_zram
    maybe_start_service
    validate_install
    print_summary
}

# Repair: fix a broken or drifted install WITHOUT forcing the big downloads.
# This is the wizard for exactly the failure that motivated it — a service unit
# that won't start because of a bad ReadWritePaths entry, or directories that
# went missing. It rewrites the unit, recreates directories with correct
# ownership, fixes the config permissions, and restarts.
do_repair() {
    MODE="repair"
    info "Repairing the existing BirdNet-Behavior install…"
    check_required_tools

    local was_active=0
    if has_systemd; then
        systemctl is-active --quiet "${SERVICE_NAME}" 2>/dev/null && was_active=1
    fi

    # Binary: only (re)download if it is actually missing.
    if [ ! -x "${INSTALL_DIR}/${BINARY_NAME}" ]; then
        warn "Binary missing — downloading it."
        local arch version
        arch="$(detect_arch)"
        check_glibc
        version="$(resolve_version)"
        install_binary "${version}" "${arch}"
    else
        success "Binary present ($("${INSTALL_DIR}/${BINARY_NAME}" --version 2>/dev/null | awk 'NR==1'))."
    fi

    create_directories
    setup_tmpfs_streaming

    if [ -f "${MODEL_DIR}/${MODEL_FILE}" ] && [ -f "${MODEL_DIR}/${LABELS_FILE}" ]; then
        success "Model present — skipping download."
    else
        warn "Model files missing — downloading."
        download_model
    fi

    # A repair run is how a station installed before the geomodel shipped picks
    # it up: download_geomodel is a no-op when both files are already there.
    download_geomodel

    write_config             # idempotent: fixes ownership/permissions, keeps content
    resolve_listen_addr      # preserve LAN bind across re-runs (env > config > unit)
    ensure_capture_backend   # RTSP stations need ffmpeg or the daemon can't start
    install_service          # rewrites the unit (this is what fixes a bad unit) + reload

    # Clear any failed / rate-limited state from prior crash loops, then bring
    # the service up with the repaired unit.
    if ! has_systemd; then
        warn "systemd is not running here — rewrote the unit but did not (re)start it."
    elif [ "${was_active}" = 1 ] || grep -qE '^(ALSA_CARD|RTSP_URL)=' "${CONFIG_FILE}" 2>/dev/null; then
        systemctl reset-failed "${SERVICE_NAME}" 2>/dev/null || true
        info "Starting the service with the repaired unit…"
        if systemctl restart "${SERVICE_NAME}"; then
            success "Service (re)started."
        else
            warn "Service still failed to start — inspect: journalctl -xeu ${SERVICE_NAME}"
        fi
    else
        warn "No audio source configured — not starting."
        warn "Edit ${CONFIG_FILE}, then: sudo systemctl start birdnet-behavior"
    fi

    validate_install
    echo
    success "Repair complete."
    echo "  Logs:  sudo journalctl -u birdnet-behavior -f"
}

# Remove the software cleanly. Data/config are kept unless the operator opts in
# (interactively, or via BIRDNET_PURGE=1). Idempotent: safe to re-run when
# nothing — or only part — is installed.
do_uninstall() {
    info "Removing BirdNet-Behavior…"

    # Record what's actually present so we report accurately and stay idempotent.
    local had_unit=0 had_binary=0
    [ -f "${SERVICE_FILE}" ] && had_unit=1
    [ -x "${INSTALL_DIR}/${BINARY_NAME}" ] && had_binary=1

    if command -v systemctl >/dev/null 2>&1; then
        # The daemon may take its TimeoutStopSec to drain; `|| true` keeps a
        # slow or already-stopped unit from aborting the uninstall.
        systemctl stop "${SERVICE_NAME}" 2>/dev/null || true
        systemctl disable "${SERVICE_NAME}" 2>/dev/null || true
        systemctl disable --now 'tmp-birdnet\x2dstream.mount' 2>/dev/null || true
    fi

    rm -f "${SERVICE_FILE}" '/etc/systemd/system/tmp-birdnet\x2dstream.mount'

    if command -v systemctl >/dev/null 2>&1; then
        systemctl daemon-reload 2>/dev/null || true
        # Clear the lingering failed/timeout state so `systemctl status` reads a
        # clean "not-found" instead of "Active: failed" after the unit is gone.
        systemctl reset-failed "${SERVICE_NAME}" 2>/dev/null || true
    fi

    rm -f "${INSTALL_DIR}/${BINARY_NAME}"
    rm -rf "${STREAM_DIR}"
    # Bundled operator manual (and its parent dir if now empty).
    rm -rf "${HELP_DIR}"
    rmdir "$(dirname "${HELP_DIR}")" 2>/dev/null || true

    if [ "${had_unit}" = 0 ] && [ "${had_binary}" = 0 ]; then
        warn "No installed service or binary found — nothing to remove."
    else
        success "Removed the service, tmpfs unit, and binary."
    fi

    remove_data_or_keep
    verify_uninstall
}

# Offer to delete the data + config (interactive, or BIRDNET_PURGE=1); otherwise
# keep them and show how to remove them later. Guards against deleting anything
# but a real "<home>/BirdNet-Behavior" data directory.
remove_data_or_keep() {
    [ -e "${DATA_DIR}" ] || [ -e "${CONFIG_DIR}" ] || return 0

    local do_purge=0
    if [ "${BIRDNET_PURGE:-0}" = "1" ]; then
        do_purge=1
    elif [ "${INTERACTIVE}" = 1 ] \
        && yesno "  Also delete ALL data — database, recordings, model (~541 MB), config?" n; then
        do_purge=1
    fi

    if [ "${do_purge}" = 1 ]; then
        if [ -z "${DATA_DIR}" ] || [ "${DATA_DIR}" = "/" ] \
            || [ "${DATA_DIR%/BirdNet-Behavior}" = "${DATA_DIR}" ]; then
            warn "Data dir ${DATA_DIR:-<unset>} looks unsafe to auto-delete — remove it manually."
        else
            rm -rf "${DATA_DIR}"
            success "Removed data directory ${DATA_DIR}."
        fi
        rm -rf "${CONFIG_DIR}"
        success "Removed config ${CONFIG_DIR}."
    else
        warn "Kept your data and config (reinstall will reuse them):"
        [ -e "${DATA_DIR}" ]   && warn "    data:   ${DATA_DIR}"
        [ -e "${CONFIG_DIR}" ] && warn "    config: ${CONFIG_DIR}"
        warn "    Remove later with:  sudo rm -rf ${DATA_DIR} ${CONFIG_DIR}"
    fi
}

# Confirm nothing BirdNet-Behavior remains, so the operator isn't left with a
# half-removed install.
verify_uninstall() {
    local problems=0
    if [ -e "${INSTALL_DIR}/${BINARY_NAME}" ]; then
        warn "binary still present: ${INSTALL_DIR}/${BINARY_NAME}"
        problems=1
    fi
    if [ -e "${SERVICE_FILE}" ]; then
        warn "service unit still present: ${SERVICE_FILE}"
        problems=1
    fi
    # Captured, not piped into `grep -q`: `systemctl list-unit-files` prints
    # hundreds of lines, grep quits at the first match, and `set -o pipefail`
    # then reports the whole pipeline failed — so "the unit is still listed"
    # read as "it is gone" and this warning was skipped. Measured at 1 in 300
    # with a 5000-line producer.
    local unit_files=""
    if command -v systemctl >/dev/null 2>&1; then
        unit_files="$(systemctl list-unit-files 2>/dev/null || true)"
    fi
    if grep -q "^${SERVICE_NAME}" <<<"${unit_files}"; then
        warn "systemd still lists ${SERVICE_NAME} — try: sudo systemctl daemon-reload"
        problems=1
    fi
    if [ "${problems}" = 0 ]; then
        success "Uninstall verified — no BirdNet-Behavior service or binary remains."
    else
        warn "Some components could not be removed (see above)."
    fi
}

# Present the choices when an existing install is found (interactive only).
# Sets SUBCOMMAND to the chosen flow.
choose_existing_action() {
    printf '\n  An existing BirdNet-Behavior install was detected (%s).\n' "$(describe_existing_install)" >/dev/tty
    printf '  What would you like to do?\n\n' >/dev/tty
    printf '    1) Update      — install the latest binary, keep all settings (default)\n' >/dev/tty
    printf '    2) Repair      — recreate dirs, fix permissions, rewrite the service unit, restart\n' >/dev/tty
    printf '    3) Reinstall   — re-download the binary, rewrite the unit/config (keeps data + model)\n' >/dev/tty
    printf '    4) Uninstall   — remove the software, keep your data\n' >/dev/tty
    printf '    5) Cancel      — do nothing\n\n' >/dev/tty
    local choice
    choice="$(ask "  Choose [1-5]" "1")"
    case "${choice}" in
        1) SUBCOMMAND="update" ;;
        2) SUBCOMMAND="repair" ;;
        3) SUBCOMMAND="reinstall" ;;
        4) SUBCOMMAND="uninstall" ;;
        *) info "Cancelled — nothing changed."; exit 0 ;;
    esac
}

# ===== installer/lib/80-summary.sh =====
# ---------------------------------------------------------------------------
# Print post-install instructions
# ---------------------------------------------------------------------------

# Print a scannable QR of the dashboard URL when possible, so a phone can open
# the dashboard without anyone typing an IP. Best-effort: needs qrencode and a
# LAN-reachable host (a localhost-only bind has nothing for another device to
# scan). The IP URL is encoded on purpose — it always resolves on the LAN,
# whereas mDNS `.local` is not universal (some phones / networks never resolve it).
print_dashboard_qr() {
    local host="$1" port="$2"
    [ "${host}" = "localhost" ] && return 0
    command -v qrencode &>/dev/null || return 0
    echo
    echo -e "  ${BOLD}Scan to open on your phone${RESET} (same Wi-Fi network):"
    qrencode -t ANSIUTF8 -m 2 "http://${host}:${port}" 2>/dev/null | sed 's/^/    /' || true
}

# Admin login. Viewing the dashboard is open; the admin panel (settings,
# software update, system controls) requires signing in.
#
# This has to work on an UPGRADE, not only on a fresh install.
# GENERATED_ADMIN_PASSWORD and CADDY_PWD_VALUE are set only during onboarding,
# and onboarding is skipped whenever a config already exists
# (`70-station.sh` returns early on `[ -f "${CONFIG_FILE}" ]`). So on every
# update/repair/reinstall of an existing station both were empty and this block
# printed NOTHING — no username, no mention that a password exists at all.
#
# That is a lockout, not a cosmetic gap. This release closes an `/admin` that
# was served unauthenticated, so an operator who has never needed the password
# since install now needs one they were shown once, months ago, in scrollback
# that is long gone — and the only route back is a root grep of a file they
# have no reason to know exists.
#
# The password itself is deliberately NOT reprinted here. It is already in the
# operator's terminal history from install time, and an upgrade is not a reason
# to spray it into scrollback again. The exact command to reveal it is printed
# instead, so the recovery path is one copy-paste rather than a search.
print_admin_login() {
    if [ -n "${GENERATED_ADMIN_PASSWORD}" ]; then
        echo
        echo -e "  ${BOLD}Admin panel login${RESET} (settings + software update — viewing is open):"
        echo -e "      username:  ${BOLD}admin${RESET}"
        echo -e "      password:  ${BOLD}${GENERATED_ADMIN_PASSWORD}${RESET}"
        echo    "      (auto-generated, saved as CADDY_PWD in ${CONFIG_FILE} — change it any time)"
        echo    "      Save it now — it is not shown again."
        return 0
    fi
    if [ -n "${CADDY_PWD_VALUE}" ]; then
        echo
        echo -e "  ${BOLD}Admin panel login${RESET} (settings + software update — viewing is open):"
        echo -e "      username:  ${BOLD}admin${RESET}"
        echo    "      password:  the one you just set."
        return 0
    fi

    # Nothing was generated or entered this run — an upgrade or a repair. Read
    # the station's own config to say something useful rather than nothing.
    if grep -qE '^[[:space:]]*CADDY_PWD[[:space:]]*=[[:space:]]*[^[:space:]#]' \
        "${CONFIG_FILE}" 2>/dev/null; then
        echo
        echo -e "  ${BOLD}Admin panel login${RESET} (settings + software update — viewing stays open):"
        echo -e "      username:  ${BOLD}admin${RESET}"
        echo    "      password:  already set on this station. To see it:"
        echo -e "        ${BOLD}sudo grep '^CADDY_PWD' ${CONFIG_FILE}${RESET}"
        echo    "      Signing in is required for the admin panel — earlier versions"
        echo    "      served it without a password. Viewing the dashboard is unchanged."
        return 0
    fi

    # No password at all. On a LAN-reachable station that means the admin panel
    # is open to anyone who can load the dashboard, which the operator should
    # hear about loudly rather than discover.
    case "${LISTEN_ADDR}" in
        127.0.0.1:* | localhost:*)
            echo
            echo "  Admin panel: no password set (CADDY_PWD). The dashboard is bound to"
            echo "  this device only, so the panel is reachable just from here."
            ;;
        *)
            loud_warn \
                "The admin panel has NO PASSWORD and the dashboard is on your network." \
                "Anyone who can reach it can change settings and update the software." \
                "Fix: set CADDY_PWD in ${CONFIG_FILE}, then:" \
                "  sudo bash install.sh repair"
            ;;
    esac
    return 0
}

print_summary() {
    local ip web_host mdns_host
    ip="$(hostname -I 2>/dev/null | awk '{print $1}' || echo 'localhost')"
    # Show the address — and port — the dashboard actually answers on, so an
    # operator who set a custom BIRDNET_LISTEN sees the right URL, not the default.
    case "${LISTEN_ADDR}" in
        127.0.0.1:* | localhost:*) web_host="localhost" ;;
        *)                         web_host="${ip}" ;;
    esac
    local web_port="${LISTEN_ADDR##*:}"
    case "${web_port}" in
        '' | *[!0-9]*) web_port="8502" ;; # no/invalid port → canonical default
    esac

    # Best-effort mDNS name. Pi OS ships avahi, so http://<hostname>.local is a
    # more durable bookmark than a DHCP-assigned IP (which can change on the next
    # lease). Only meaningful when the dashboard is exposed beyond localhost;
    # clients without an mDNS resolver fall back to the IP shown beside it.
    mdns_host=""
    if [ "${web_host}" != "localhost" ]; then
        local short
        short="$(hostname -s 2>/dev/null || true)"
        [ -n "${short}" ] && mdns_host="${short}.local"
    fi

    local headline="Installation complete!"
    case "${MODE}" in
        update)    headline="Update complete!" ;;
        reinstall) headline="Reinstall complete!" ;;
    esac

    echo
    echo -e "${BOLD}${GREEN}${headline}${RESET}"
    echo
    echo -e "  ${BOLD}Binary:${RESET}  ${INSTALL_DIR}/${BINARY_NAME}"
    echo -e "  ${BOLD}Config:${RESET}  ${CONFIG_FILE}"
    echo -e "  ${BOLD}Data:${RESET}    ${DATA_DIR}"
    # Lead with the IP address: it works for every device on the LAN. The mDNS
    # `.local` name is friendlier but NOT universal — some phones and networks
    # don't resolve it — so it is shown as a clearly-secondary convenience, never
    # the only address the operator is handed.
    echo -e "  ${BOLD}Web UI:${RESET}  http://${web_host}:${web_port}   (works from any device on your network)"
    [ -n "${mdns_host}" ] && echo -e "           http://${mdns_host}:${web_port}   (friendlier name — works on most devices, but not all)"
    print_dashboard_qr "${web_host}" "${web_port}"
    echo
    if systemctl is-active --quiet birdnet-behavior.service 2>/dev/null; then
        echo -e "${GREEN}Your dashboard is live${RESET} — open a web browser to:  ${BOLD}http://${web_host}:${web_port}${RESET}"
        [ "${web_host}" != "localhost" ] && echo "  (reachable from any device on your network)"
        # Live, but a station with no audio source won't detect anything yet.
        # Point the operator at the in-dashboard setup wizard (and the config
        # fallback) so it's clear why no birds are showing up.
        if ! config_has_audio_source; then
            echo
            echo "  No audio source yet, so no birds will be detected. Open the dashboard"
            echo "  to pick a microphone in the setup wizard — or set ALSA_CARD / RTSP_URL"
            echo "  in ${CONFIG_FILE} and:  sudo systemctl restart birdnet-behavior"
        fi
        # Live and listening, but with no coordinates the species filter cannot
        # run — so the station reports birds from the wrong continent and looks
        # like a bad model. Silence here was the whole problem: unlike a missing
        # microphone, this failure produces detections, just wrong ones.
        if ! config_has_location; then
            echo
            echo "  No station location set. Without it every species in the model stays a"
            echo "  candidate, so expect detections that don't belong in your area. Set it"
            echo "  in the dashboard's setup wizard (it can auto-detect), or add LATITUDE"
            echo "  and LONGITUDE to ${CONFIG_FILE} and:  sudo systemctl restart birdnet-behavior"
        fi
    else
        echo -e "${BOLD}Next steps:${RESET}"
        echo "  1. Set an audio source (edit as root):  sudo nano ${CONFIG_FILE}"
        echo "       ALSA_CARD=plughw:1,0      (ALSA microphone)"
        echo "       RTSP_URL=rtsp://…         (RTSP camera)"
        echo
        echo "  2. Set LATITUDE and LONGITUDE. Without them the species filter cannot"
        echo "     run and you will get detections from the wrong part of the world."
        echo
        echo "  3. sudo systemctl start birdnet-behavior"
        echo "  4. Open a web browser to  http://${web_host}:${web_port}"
    fi

    print_admin_login
    echo
    echo "  Logs:  sudo journalctl -u birdnet-behavior -f"
    echo
}

# ===== installer/lib/82-zram.sh =====
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
        pkg_install util-linux || {
            warn "Could not install util-linux ($(pkg_install_hint util-linux)). Skipping ZRAM setup."
            return 0
        }
    fi

    local mem_bytes
    mem_bytes="$(awk '/MemTotal/ {print $2 * 1024}' /proc/meminfo)"
    local zram_size=$(( mem_bytes / 2 ))   # 50% of physical RAM

    # Load the zram kernel module
    local loaded_modules
    loaded_modules="$(lsmod 2>/dev/null || true)"
    if ! grep -q '^zram' <<<"${loaded_modules}"; then
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

    # Persist across reboots via a systemd service unit.
    #
    # ExecStop deserves a note. It used to begin `swapoff -a`, which is
    # documented as "disable all swaps from /proc/swaps" — every swap on the
    # machine, not the zram device this unit made. Raspberry Pi OS enables
    # dphys-swapfile by default, so stopping this unit (on shutdown, on
    # `systemctl stop zram-swap`, during an uninstall) silently switched off the
    # operator's real swap on exactly the low-RAM boards this unit exists to
    # help. It then piped device paths into `rmmod`, which takes a module name,
    # so `rmmod /dev/zram0` failed on every run and `|| true` hid it.
    #
    # Now: only /dev/zram* are swapped off, and the module is unloaded by name.
    # This still cannot distinguish a zram device made by another provider
    # (zram-tools) from ours — the unit records no device id — but at shutdown
    # that is the difference between touching zram and touching everything.
    local zram_service="/etc/systemd/system/zram-swap.service"
    cat > "${zram_service}" << EOF
[Unit]
Description=ZRAM compressed swap for BirdNet-Behavior
After=multi-user.target

[Service]
Type=oneshot
RemainAfterExit=yes
ExecStart=/bin/sh -c 'modprobe zram num_devices=1 && zramctl --find --size ${zram_size} --algorithm lz4 | xargs -I{} sh -c "mkswap {} && swapon --priority 100 {}"'
ExecStop=/bin/sh -c 'for d in /dev/zram*; do [ -b "\$d" ] && swapoff "\$d" 2>/dev/null; done; rmmod zram 2>/dev/null || true'

[Install]
WantedBy=multi-user.target
EOF

    if has_systemd; then
        systemctl daemon-reload
        systemctl enable zram-swap.service &>/dev/null
        success "ZRAM swap service installed and enabled (persists across reboots)."
    else
        success "ZRAM swap unit written (enable it with systemctl on a systemd host)."
    fi
}

# ===== installer/lib/85-macos.sh =====
# ---------------------------------------------------------------------------
# macOS (Apple Silicon) install path
#
# The Linux flow above is systemd-specific and would break partway on macOS.
# macOS instead gets a per-user launchd LaunchAgent (no sudo), the
# aarch64-apple-darwin prebuilt when a release publishes one, and clear
# from-source guidance until then — so a Mac user who runs this script never
# ends up half-installed.
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
# BirdNet-Behavior config (macOS). Set your coordinates and a mic device.
#
# LATITUDE/LONGITUDE are left commented deliberately. They used to be written
# as 0.0/0.0, which is not "unset" — it is Null Island in the Gulf of Guinea,
# and the metadata model would filter this station's species list for that
# spot. An unset location skips occurrence filtering entirely, which is honest;
# a wrong one produces a confident species list for the wrong continent, and
# `config_has_location` would report the station as configured.
SITENAME=My Backyard
# LATITUDE=51.5074
# LONGITUDE=-0.1278
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

# ===== installer/lib/90-args.sh =====
# ---------------------------------------------------------------------------
# Argument parsing and usage
# ---------------------------------------------------------------------------

usage() {
    cat <<EOF
BirdNet-Behavior installer

Linux / Raspberry Pi (installs a systemd service, so it needs root):
  curl -fsSL https://raw.githubusercontent.com/${REPO}/main/install.sh | sudo bash
  curl -fsSL https://raw.githubusercontent.com/${REPO}/main/install.sh | sudo bash -s -- --version X.Y.Z

From a saved copy of this script:
  sudo bash install.sh [--version X.Y.Z]

Commands (auto-offered as a menu when an existing install is detected):
  install        Fresh install (the default when nothing is installed).
  update         Install the latest (or --version) binary; keep all settings.
  repair         Recreate directories, fix permissions, rewrite the systemd
                 unit, and restart. Fixes a service that won't start without
                 re-downloading anything.
  reinstall      Re-download the binary and rewrite the unit/config
                 (your data and the model are preserved).
  uninstall      Remove the software (binary, service, tmpfs unit); keep data.

Options:
  -v, --version X.Y.Z   Install a specific release (default: latest stable).
                        The VERSION environment variable is still honoured too.
      --noninteractive  Don't prompt; auto-detect audio and leave location
                        unset (also implied by BIRDNET_NONINTERACTIVE=1 or no TTY).
                        With an existing install this implies 'update'.
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
            install | update | repair | reinstall | uninstall)
                [ -z "${SUBCOMMAND}" ] || fatal "Specify only one command (got '${SUBCOMMAND}' and '$1')."
                SUBCOMMAND="$1"
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

# ===== installer/lib/95-main.sh =====
# ---------------------------------------------------------------------------
# Main entry point
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

    require_root
    detect_existing_install

    # No explicit command: a fresh box installs; an existing one offers the
    # menu interactively, or silently updates when non-interactive (preserving
    # the historical `curl | sudo bash` auto-upgrade behaviour for automation).
    if [ -z "${SUBCOMMAND}" ]; then
        if [ "${EXISTING_INSTALL}" = 1 ]; then
            if [ "${INTERACTIVE}" = 1 ]; then
                choose_existing_action
            else
                info "Existing install detected — updating (non-interactive)."
                SUBCOMMAND="update"
            fi
        else
            SUBCOMMAND="install"
        fi
    fi

    MODE="${SUBCOMMAND}"
    case "${SUBCOMMAND}" in
        install | update | reinstall) do_install ;;
        repair)                       do_repair ;;
        uninstall)                    do_uninstall ;;
        *)                            fatal "Unknown command: ${SUBCOMMAND}" ;;
    esac
}

main "$@"
