# ---------------------------------------------------------------------------
# Global configuration and shared state
# ---------------------------------------------------------------------------
set -euo pipefail

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
SERVICE_NAME="birdnet-behavior.service"
SERVICE_USER="${SUDO_USER:-${USER:-$(id -un)}}"
# Bind localhost by default — the admin dashboard can change settings and update
# software, so it must not reach the LAN without an explicit (ideally password-
# protected) opt-in. Override with BIRDNET_LISTEN= or the interactive prompt.
LISTEN_ADDR="${BIRDNET_LISTEN:-127.0.0.1:8502}"

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
