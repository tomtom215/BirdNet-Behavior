#!/usr/bin/env bash
# =============================================================================
# BirdNet-Behavior — Docker quick start bootstrap
#
# One-command setup for non-technical users who just want to spin up
# BirdNet-Behavior without editing config files or reading the manual.
#
# Usage:
#   bash <(curl -fsSL https://raw.githubusercontent.com/tomtom215/BirdNet-Behavior/main/quickstart.sh)
#
# What this script does — you answer 2-3 questions, it does the rest:
#   1. Verifies Docker Engine + Compose plugin are installed and usable,
#      with actionable remediation if they're not.
#   2. Creates a small working directory (default: ~/birdnet-behavior/)
#      that holds your .env and compose files. Recordings and the database
#      live in a Docker-managed volume, not here.
#   3. Downloads the canonical compose files from GitHub.
#   4. Auto-detects your audio source:
#        • USB/ALSA capture card  → plughw:<card>,<device>
#        • PulseAudio / PipeWire socket  → default source
#        • Otherwise prompts you for an RTSP stream URL
#   5. Asks for your station coordinates (or auto-detects via opt-in IP
#      geolocation through ipapi.co — off by default, you have to say yes).
#   6. Asks whether to enable DuckDB behavioral analytics.
#   7. Writes a minimal 6-line .env with only your chosen values.
#   8. Starts the container with the matching compose overlay.
#   9. Tails the logs so you can watch the first-run model download, and
#      stops tailing automatically as soon as the web server reports healthy.
#  10. Prints the dashboard URL and your LAN IP.
#
# Requirements: bash, curl, docker, docker compose. That's it.
# =============================================================================

set -euo pipefail

REPO="tomtom215/BirdNet-Behavior"
RAW_BASE="https://raw.githubusercontent.com/${REPO}/main"
COMPOSE_FILES="docker-compose.yml docker-compose.alsa.yml docker-compose.pulse.yml"
DEFAULT_DIR="${HOME}/birdnet-behavior"
WEB_PORT=8502
MODEL_SIZE_HINT="541 MB"

# ---------------------------------------------------------------------------
# Presentation helpers
# ---------------------------------------------------------------------------
if [ -t 1 ]; then
    RED=$'\033[0;31m'; GRN=$'\033[0;32m'; YEL=$'\033[1;33m'
    BLU=$'\033[0;34m'; BLD=$'\033[1m';    RST=$'\033[0m'
else
    RED=""; GRN=""; YEL=""; BLU=""; BLD=""; RST=""
fi

say()   { printf '%s\n' "$*"; }
info()  { printf '%s[i]%s %s\n' "$BLU" "$RST" "$*"; }
ok()    { printf '%s[OK]%s %s\n' "$GRN" "$RST" "$*"; }
warn()  { printf '%s[!]%s %s\n'  "$YEL" "$RST" "$*" >&2; }
fail()  { printf '%s[x]%s %s\n'  "$RED" "$RST" "$*" >&2; exit 1; }
hdr()   { printf '\n%s=== %s ===%s\n\n' "$BLD" "$*" "$RST"; }

# Prompt for a value with a default. Reads from /dev/tty so it works when
# the script is piped through `bash <(curl ...)`.
ask() {
    local prompt="$1" default="${2:-}" reply
    if [ -n "$default" ]; then
        printf '%s (default: %s%s%s): ' "$prompt" "$BLD" "$default" "$RST" >/dev/tty
    else
        printf '%s: ' "$prompt" >/dev/tty
    fi
    read -r reply </dev/tty || reply=""
    printf '%s' "${reply:-$default}"
}

# Yes/no prompt with a default. Returns 0 for yes, 1 for no.
yesno() {
    local prompt="$1" default="${2:-y}" reply hint="[Y/n]"
    [ "$default" = "n" ] && hint="[y/N]"
    printf '%s %s ' "$prompt" "$hint" >/dev/tty
    read -r reply </dev/tty || reply=""
    reply="${reply:-$default}"
    case "${reply:0:1}" in y|Y) return 0 ;; *) return 1 ;; esac
}

# ---------------------------------------------------------------------------
# Preflight — check everything we depend on before touching the filesystem
# ---------------------------------------------------------------------------
hdr "Checking prerequisites"

command -v curl >/dev/null 2>&1 \
    || fail "curl is required but not installed. Install it with your package manager and try again."

if ! command -v docker >/dev/null 2>&1; then
    fail "Docker is not installed.
        Install Docker Engine: https://docs.docker.com/engine/install/
        On Raspberry Pi OS:    curl -fsSL https://get.docker.com | sudo sh"
fi

if ! docker version >/dev/null 2>&1; then
    fail "Docker is installed but not reachable. Most common causes:
        1. The daemon is not running.
               sudo systemctl start docker
        2. Your user is not in the 'docker' group (required to run Docker
           without sudo).
               sudo usermod -aG docker \$USER
           then log out and log back in (or run 'newgrp docker')."
fi

if ! docker compose version >/dev/null 2>&1; then
    fail "Docker Compose plugin is missing. Install it:
        https://docs.docker.com/compose/install/
        On most distros the package is called 'docker-compose-plugin'."
fi

DOCKER_VER=$(docker version --format '{{.Server.Version}}' 2>/dev/null || echo unknown)
COMPOSE_VER=$(docker compose version --short 2>/dev/null || echo unknown)
ok "Docker Engine: ${DOCKER_VER}"
ok "Docker Compose: ${COMPOSE_VER}"

# Disk space — we need ~1 GB (541 MB model + headroom for recordings).
AVAIL_MB=$(df -BM --output=avail "$HOME" 2>/dev/null | tail -1 | tr -dc '0-9' || echo 0)
if [ "${AVAIL_MB:-0}" -lt 1000 ]; then
    warn "Less than 1 GB free in your home directory (have: ${AVAIL_MB} MB)."
    warn "The BirdNET+ model alone is ${MODEL_SIZE_HINT}."
    yesno "Continue anyway?" n || exit 1
else
    ok "Disk space: $(( AVAIL_MB / 1024 )) GB free in \$HOME"
fi

# Port check — if :8502 is already bound, the container will fail to start.
if command -v ss >/dev/null 2>&1 && ss -tlnH "sport = :${WEB_PORT}" 2>/dev/null | grep -q .; then
    warn "Port ${WEB_PORT} is already in use on this host:"
    ss -tlnH "sport = :${WEB_PORT}" 2>/dev/null | sed 's/^/    /' >&2
    warn "You can change the host port later in .env:  BIRDNET_PORT=8080"
    yesno "Continue anyway?" n || exit 1
fi

# ---------------------------------------------------------------------------
# Working directory
# ---------------------------------------------------------------------------
hdr "Where to install"
say "We'll create a small directory to hold your .env and compose files."
say "Your recordings, database, and cached model live in a Docker-managed"
say "volume called 'birdnet-data', not in this directory — so it stays tiny"
say "(< 50 KB) and you can delete it without losing data."
say ""

WORK_DIR=$(ask "Working directory" "$DEFAULT_DIR")
WORK_DIR=${WORK_DIR/#\~/$HOME}   # expand ~ if the user typed one
mkdir -p "$WORK_DIR"
cd "$WORK_DIR"

if [ -f .env ] || [ -f docker-compose.yml ]; then
    warn "This directory already contains a BirdNet-Behavior setup:"
    [ -f .env ]               && say "    $(pwd)/.env"
    [ -f docker-compose.yml ] && say "    $(pwd)/docker-compose.yml"
    say ""
    if ! yesno "Overwrite and start over?" n; then
        info "Leaving the existing setup alone."
        info "Start it with:   cd $(pwd) && docker compose up -d"
        exit 0
    fi
fi

# ---------------------------------------------------------------------------
# Download compose files
# ---------------------------------------------------------------------------
hdr "Downloading compose files"

for f in $COMPOSE_FILES; do
    info "Fetching $f"
    if ! curl -fsSL --retry 3 --retry-delay 2 -o "$f" "$RAW_BASE/$f"; then
        fail "Could not download $f from $RAW_BASE/$f
        Check your internet connection and try again."
    fi
done
ok "Compose files ready in $(pwd)"

# ---------------------------------------------------------------------------
# Audio source — auto-detect, then confirm with the user
# ---------------------------------------------------------------------------
hdr "Audio source"

AUDIO_KIND=""    # alsa | pulse | rtsp | none
AUDIO_VALUE=""

# Try ALSA first (most Raspberry Pi USB mic setups).
if command -v arecord >/dev/null 2>&1; then
    first_card=$(arecord -l 2>/dev/null | awk '/^card/{print $2; exit}' | tr -d ':')
    first_dev=$(arecord -l 2>/dev/null \
        | awk '/^card/ {
               for (i=1; i<=NF; i++) if ($i=="device") { v=$(i+1); gsub(":","",v); print v; exit }
           }')
    if [ -n "${first_card:-}" ]; then
        candidate="plughw:${first_card},${first_dev:-0}"
        say "Found a USB/ALSA capture device:"
        arecord -l 2>/dev/null | grep '^card' | sed 's/^/    /'
        say ""
        if yesno "Use '${candidate}' as the audio source?" y; then
            AUDIO_KIND="alsa"
            AUDIO_VALUE="$candidate"
        fi
    fi
fi

# PulseAudio / PipeWire socket (desktop Linux).
if [ -z "$AUDIO_KIND" ] && [ -S "/run/user/$(id -u)/pulse/native" ]; then
    say "Found a PulseAudio/PipeWire socket at /run/user/$(id -u)/pulse/native"
    if yesno "Use the default PulseAudio/PipeWire source?" y; then
        AUDIO_KIND="pulse"
        AUDIO_VALUE="default"
    fi
fi

# Fall back to RTSP.
if [ -z "$AUDIO_KIND" ]; then
    say "No local microphone selected — you can use an RTSP stream instead."
    say "Examples:"
    say "    rtsp://192.168.1.50:554/ch0_0.h264"
    say "    rtsp://user:pass@camera.lan:554/stream"
    say ""
    rtsp_url=$(ask "RTSP URL (press Enter to skip audio entirely)" "")
    if [ -n "$rtsp_url" ]; then
        AUDIO_KIND="rtsp"
        AUDIO_VALUE="$rtsp_url"
    else
        AUDIO_KIND="none"
        warn "No audio source. The dashboard will still come up but no"
        warn "detections will be produced until you set one in .env later."
    fi
fi

# ---------------------------------------------------------------------------
# Station location — manual entry, with opt-in IP-based auto-detect
# ---------------------------------------------------------------------------
hdr "Station location"
say "Your coordinates are used for the sunrise/sunset recording schedule,"
say "the species frequency filter, and BirdWeather uploads."
say ""

LAT=""
LON=""

if yesno "Auto-detect your location from your public IP? (sends a request to ipapi.co)" n; then
    info "Querying ipapi.co…"
    geo=$(curl -fsSL --max-time 5 https://ipapi.co/json/ 2>/dev/null || true)
    if [ -n "$geo" ]; then
        LAT=$(printf '%s' "$geo" | grep -o '"latitude":[^,}]*'  | head -1 | cut -d: -f2 | tr -d ' "')
        LON=$(printf '%s' "$geo" | grep -o '"longitude":[^,}]*' | head -1 | cut -d: -f2 | tr -d ' "')
        city=$(printf '%s' "$geo" | grep -o '"city":"[^"]*"' | head -1 | cut -d: -f2 | tr -d '"')
        if [ -n "$LAT" ] && [ -n "$LON" ]; then
            info "Detected ${city:-(unknown city)}: ${LAT}, ${LON}"
            if ! yesno "Use these coordinates?" y; then
                LAT=""; LON=""
            fi
        else
            warn "ipapi.co did not return usable coordinates — please enter them manually."
        fi
    else
        warn "Could not reach ipapi.co — please enter coordinates manually."
    fi
fi

if [ -z "$LAT" ]; then
    say ""
    say "Tip: open https://www.openstreetmap.org, right-click your station,"
    say "     and choose 'Show address' to read off the coordinates."
    say ""
    LAT=$(ask "Latitude  (e.g. 42.3601)"  "")
    LON=$(ask "Longitude (e.g. -71.0589)" "")
fi

if [ -z "$LAT" ] || [ -z "$LON" ]; then
    warn "No coordinates set — solar schedule and species frequency filter"
    warn "will be disabled. You can add them to .env later."
fi

# ---------------------------------------------------------------------------
# Image variant (analytics on/off)
# ---------------------------------------------------------------------------
hdr "Behavioral analytics"
say "The 'analytics' image adds DuckDB behavioral analytics (activity"
say "sessions, resident-vs-migrant classification, dawn chorus validation,"
say "species co-occurrence). It's about 80 MB larger than the standard image;"
say "otherwise identical. You can switch later by editing BIRDNET_IMAGE_TAG."
say ""

if yesno "Enable behavioral analytics?" n; then
    IMAGE_TAG="latest-analytics"
else
    IMAGE_TAG="latest"
fi

# ---------------------------------------------------------------------------
# Write the minimal .env
# ---------------------------------------------------------------------------
hdr "Writing your .env"

{
    printf '# Generated by quickstart.sh on %s\n' "$(date -u +'%Y-%m-%d %H:%M UTC')"
    printf '# Edit any value and run:  docker compose up -d\n'
    printf '\n'
    printf '# --- Station location ---\n'
    printf 'BIRDNET_LATITUDE=%s\n'  "$LAT"
    printf 'BIRDNET_LONGITUDE=%s\n' "$LON"
    printf '\n'
    printf '# --- Audio source ---\n'
    case "$AUDIO_KIND" in
        alsa)  printf 'BIRDNET_ALSA_DEVICE=%s\n'     "$AUDIO_VALUE" ;;
        pulse) printf 'BIRDNET_PIPEWIRE_DEVICE=%s\n' "$AUDIO_VALUE" ;;
        rtsp)  printf 'BIRDNET_RTSP_URL=%s\n'        "$AUDIO_VALUE" ;;
        none)  printf '# (none set — add BIRDNET_ALSA_DEVICE / BIRDNET_RTSP_URL / BIRDNET_PIPEWIRE_DEVICE here)\n' ;;
    esac
    printf '\n'
    printf '# --- Image variant: latest | latest-analytics ---\n'
    printf 'BIRDNET_IMAGE_TAG=%s\n' "$IMAGE_TAG"
} > .env

ok "Wrote $(pwd)/.env ($(wc -l < .env) lines)"

# ---------------------------------------------------------------------------
# Start the stack
# ---------------------------------------------------------------------------
hdr "Starting BirdNet-Behavior"

case "$AUDIO_KIND" in
    alsa)  COMPOSE_ARGS=( -f docker-compose.yml -f docker-compose.alsa.yml  ) ;;
    pulse) COMPOSE_ARGS=( -f docker-compose.yml -f docker-compose.pulse.yml ) ;;
    *)     COMPOSE_ARGS=( -f docker-compose.yml ) ;;
esac

info "Command:  docker compose ${COMPOSE_ARGS[*]} up -d"
say ""

if ! docker compose "${COMPOSE_ARGS[@]}" up -d; then
    fail "docker compose up failed. Check the output above for details.
        Common fixes:
          • make sure the image tag exists:  docker pull ghcr.io/tomtom215/birdnet-behavior:${IMAGE_TAG}
          • check port ${WEB_PORT} is free:  ss -tlnp | grep ${WEB_PORT}
          • re-run with verbose logs:        docker compose ${COMPOSE_ARGS[*]} up  (no -d)"
fi

ok "Container started"

# ---------------------------------------------------------------------------
# Wait for health, streaming the container logs in the meantime
# ---------------------------------------------------------------------------
hdr "Waiting for the web server to come up"
say "On first run, the container downloads the BirdNET+ V3.0 model"
say "(${MODEL_SIZE_HINT}) from Zenodo. That takes:"
say "    • fibre:           ~1 min"
say "    • home broadband:  5-15 min"
say ""
say "Streaming the container logs below — press Ctrl+C at any time to detach."
say "(The container keeps running in the background either way.)"
say ""

docker compose "${COMPOSE_ARGS[@]}" logs -f --no-log-prefix birdnet &
TAIL_PID=$!

# Clean up the log tail no matter how we exit.
cleanup() { kill "$TAIL_PID" 2>/dev/null || true; wait "$TAIL_PID" 2>/dev/null || true; }
trap cleanup EXIT INT TERM

# Poll the container's health endpoint. Up to 30 minutes — well past the
# 15 min start_period of the compose healthcheck — so even a painfully slow
# connection finishes before we give up.
READY=0
for _ in $(seq 1 180); do
    sleep 10
    if curl -fsS --max-time 3 "http://127.0.0.1:${WEB_PORT}/api/v2/health" >/dev/null 2>&1; then
        READY=1
        break
    fi
done

cleanup
trap - EXIT INT TERM

# ---------------------------------------------------------------------------
# All done
# ---------------------------------------------------------------------------
hdr "All set"

# Best-effort LAN IP detection (Linux only; harmless elsewhere).
LAN_IP=$(hostname -I 2>/dev/null | awk '{print $1}')
[ -z "${LAN_IP:-}" ] && LAN_IP="localhost"

say ""
if [ "$READY" = "1" ]; then
    ok "Web server is up and healthy."
else
    warn "Web server did not report healthy within the timeout."
    warn "It may still be downloading the model — that's fine."
    warn "Check progress with:  cd $(pwd) && docker compose logs -f birdnet"
fi
say ""
say "  ${BLD}Dashboard:${RST}   http://${LAN_IP}:${WEB_PORT}"
if [ "$LAN_IP" != "localhost" ]; then
say "               http://localhost:${WEB_PORT}  (from this machine)"
fi
say ""
say "  ${BLD}Working dir:${RST}   $(pwd)"
say "  ${BLD}Data volume:${RST}   birdnet-data  (docker volume inspect birdnet-data)"
say ""
say "  ${BLD}Logs:${RST}     cd $(pwd) && docker compose logs -f birdnet"
say "  ${BLD}Stop:${RST}     cd $(pwd) && docker compose ${COMPOSE_ARGS[*]} down"
say "  ${BLD}Update:${RST}   cd $(pwd) && docker compose ${COMPOSE_ARGS[*]} pull && docker compose ${COMPOSE_ARGS[*]} up -d"
say ""
say "Open the URL in a browser — detections appear as soon as the first bird"
say "call is heard after the model finishes loading."
say ""
