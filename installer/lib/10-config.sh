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
# The journald drop-in installed beside the unit (OB-15 / PS-19 / OP-5): a
# persistent, bounded journal, so a reboot does not erase the evidence of
# what caused it and the journal cannot fill the card.
JOURNALD_DROPIN="/etc/systemd/journald.conf.d/birdnet-behavior.conf"
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

# Rough free-space floor for a fresh install: the model download + binary + DB
# headroom. Below this we warn (model download would otherwise fail mid-way
# with a less obvious error). It depends on the classifier, so
# select_classifier (52-classifier.sh) sets it: 600 for BirdNET V2.4 and 900
# for the BirdNET+ V3.0 preview (~541 MB). V2.4's is not about its ~85 MB of
# files: the binary that installs them refuses unless the model's size PLUS a
# 512 MiB margin is free (FREE_SPACE_MARGIN in model_catalog.rs), 564 MiB for
# the 54 MB model.
REQUIRED_FREE_MB=900

# Set to 1 by main() when an already-running service is stopped for an upgrade,
# so we know to restart it afterwards rather than leave it down.
SERVICE_WAS_RUNNING=0

# ---------------------------------------------------------------------------
# Classifiers
#
# Two are supported. `--classifier` (or BIRDNET_CLASSIFIER) picks one for a new
# install; select_classifier in 52-classifier.sh copies the chosen one's names
# into the unprefixed CLASSIFIER / MODEL_FILE / LABELS_FILE / FILTER_ID that the
# rest of the installer reads.
#
#   birdnet-v24  BirdNET V2.4 + its range (occurrence) filter v2. THE DEFAULT.
#                The network BirdNET-Pi and birdnet-go run, so a station that
#                installs with no customisation agrees with them about what it
#                heard (docs/book/guides/parity.md says what was measured).
#   birdnet-v3   BirdNET+ V3.0 preview + the BirdNET geomodel. Opt-in. A
#                different network (4.5 s windows at 32 kHz, calibrated
#                probabilities, ~11 500 species): its detections differ from
#                BirdNET-Pi's by design.
#
# A classifier and its occurrence filter are a pair and are never mixed: the
# V2.4 range filter is indexed like the V2.4 labels (by position) and means
# nothing to another network; the geomodel is matched by name through its own
# label file.
#
# Every model file is installed by the binary, not by this script (MDL-09):
# `birdnet-behavior --install-model <id>` for each of the four ids below. The
# digests, the sizes, the origins (this project's release first, then the
# publisher's), the retries, the resume and the timeouts all live in
# crates/birdnet-integrations/src/model_catalog.rs, and the binary verifies
# every file against them and renames it into place only when it matches.
# Nothing about the bytes is pinned here — no digest, no URL, no size — so there
# is no second copy to go stale. What IS here is the names the binary gives the
# files, so the config can point at them, and the names this installer used to
# give the V3.0 files itself, so an existing station's config can be moved onto
# the new names (the binary adopts a verified file under an old name in place,
# without downloading it again). installer/test/model-pins-match-catalogue.sh
# holds every name to the catalogue, and fails if a digest or a URL of any
# catalogue file appears in this file or in the container's entrypoint.
# ---------------------------------------------------------------------------
CLASSIFIER_V24="birdnet-v24"
CLASSIFIER_V3="birdnet-v3"
DEFAULT_CLASSIFIER="${CLASSIFIER_V24}"

# What the operator asked for (--classifier, else BIRDNET_CLASSIFIER); empty
# means "no preference". resolve_classifier turns it, together with whatever
# is already installed, into CLASSIFIER.
CLASSIFIER_REQUEST="${BIRDNET_CLASSIFIER:-}"

# BirdNET V2.4 (the default): FP32 ONNX (~54 MB), labels the binary carries
# (BirdNET-Pi's English names on V2.4's class order — the file birdnet-go
# publishes names 460 of 6 522 classes differently, which would split a species
# in two the day a BirdNET-Pi history is imported), and the range filter v2
# (~30 MB).
V24_MODEL_ID="birdnet-v24"
V24_RANGE_ID="birdnet-v24-range-v2"
V24_MODEL_FILE="birdnet-v24.onnx"
V24_LABELS_FILE="birdnet-v24_labels.txt"
V24_RANGE_FILE="birdnet-v24-range-v2.onnx"

# BirdNET+ V3.0 preview (opt-in; what every install before V2.4 became the
# default runs): FP32 ONNX (~541 MB) + labels, and the BirdNET geomodel V3.0.2
# (~14 MB) + its own labels, which score a different species list (12 012
# against 11 560) and are matched to the classifier's by name.
V3_MODEL_ID="birdnet-v3"
V3_GEOMODEL_ID="birdnet-v3-geomodel"
V3_MODEL_FILE="birdnet-v3.onnx"
V3_LABELS_FILE="birdnet-v3_labels.csv"
GEOMODEL_FILE="birdnet-v3-geomodel.onnx"
GEOMODEL_LABELS_FILE="birdnet-v3-geomodel_labels.txt"
# The names this installer gave those four files before the binary installed
# them (the publishers' own names). A station's config may name them; the
# update moves it onto the names above once the binary has adopted the files.
V3_LEGACY_MODEL_FILE="BirdNET+_V3.0-preview3_Global_11K_FP32.onnx"
V3_LEGACY_LABELS_FILE="BirdNET+_V3.0-preview3_Global_11K_Labels.csv"
GEOMODEL_LEGACY_FILE="BirdNET+_Geomodel_V3.0.2_Global_12K_FP32.onnx"
GEOMODEL_LEGACY_LABELS_FILE="BirdNET+_Geomodel_V3.0.2_Global_12K_Labels.txt"

# ---------------------------------------------------------------------------
# When the classifier cannot be installed (OOB-20; decision 7 of 2026-11-25).
#
# A failed classifier download used to be `fatal` before the config and the
# service existed, leaving a binary and nothing else. Now the binary retries a
# bounded number of times, and if it still fails the install FINISHES — the
# config, the unit, the dashboard — and says loudly, last thing on the screen,
# that the station is not detecting and how to install the model. The run then
# exits with EXIT_NO_CLASSIFIER, so a script can tell "installed, not
# detecting" from success (0) and from a run that stopped (1, or any other
# status a failing command gives under `set -e`). 75 is sysexits' EX_TEMPFAIL:
# "a temporary failure; try again later", which is exactly the remedy.
#
#   BIRDNET_MODEL_RETRY_WAITS  seconds to wait before each further round of a
#                              model's origins, space separated: "30 90" is
#                              three rounds over two minutes. "" is one round.
#                              Read by the binary, which does the retrying for
#                              every model (each round resumes what the last
#                              one received); tests set "0 0".
#   BIRDNET_LOCAL_MODEL        (or --local-model PATH) install the classifier
#                              from a file already on this machine instead of
#                              downloading it. The binary checks it against the
#                              same sha256 and size as a download and refuses a
#                              file that does not match. For an offline install.
# BIRDNET_SKIP_MODEL=1 is not a failure: the operator asked for it, and it
# still exits 0.
# ---------------------------------------------------------------------------
EXIT_NO_CLASSIFIER=75
MODEL_RETRY_WAITS="${BIRDNET_MODEL_RETRY_WAITS-30 90}"
LOCAL_MODEL="${BIRDNET_LOCAL_MODEL:-}"
# Set by classifier_not_installed (55-model.sh); read by the summary and by the
# exit status of install and repair.
CLASSIFIER_MISSING=0
CLASSIFIER_MISSING_WHY=""

# What the rest of the installer reads: the SELECTED classifier's files. Empty
# until select_classifier runs (52-classifier.sh runs it with the default as
# soon as it is loaded, and main re-runs it once it knows better).
CLASSIFIER=""
CLASSIFIER_NAME=""
MODEL_FILE=""
LABELS_FILE=""
FILTER_ID=""
MODEL_SIZE_HINT=""
# One line the final summary prints under "Classifier:", set by resolve_classifier.
CLASSIFIER_SUMMARY=""

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
