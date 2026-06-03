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
