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
    for t in tar sha256sum systemctl install mkdir awk grep sed; do
        command -v "${t}" &>/dev/null || missing+=("${t}")
    done

    if [ "${#missing[@]}" -gt 0 ]; then
        error "Missing required tool(s): ${missing[*]}"
        fatal "Install the missing package(s) and re-run. On Debian/Pi OS: sudo apt-get install coreutils tar curl"
    fi

    # Soft dependencies — note them but keep going.
    command -v getent  &>/dev/null || warn "getent not found — falling back to default data paths."
    command -v findmnt &>/dev/null || warn "findmnt not found — cannot confirm /tmp is tmpfs."
    command -v arecord &>/dev/null || true   # only needed for ALSA auto-detect
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

# RTSP capture runs through ffmpeg (so does the macOS microphone path). A
# station configured for RTSP without ffmpeg fails the doctor preflight and the
# service never starts, so make sure ffmpeg is present — installing it when we
# can. Called by the install/repair flows AFTER the config is written/known
# (an ALSA microphone on Linux uses arecord and needs no ffmpeg).
ensure_capture_backend() {
    local rtsp=0
    if [ -n "${RTSP_URL_VALUE}" ]; then
        rtsp=1
    elif [ -f "${CONFIG_FILE}" ] \
        && grep -qE '^[[:space:]]*RTSP_URL[[:space:]]*=[[:space:]]*[^[:space:]#]' "${CONFIG_FILE}"; then
        rtsp=1
    fi
    [ "${rtsp}" = 1 ] || return 0

    if command -v ffmpeg &>/dev/null; then
        success "ffmpeg present — RTSP capture backend OK."
        return 0
    fi

    warn "RTSP source configured but ffmpeg is not installed (required for RTSP capture)."
    if command -v apt-get &>/dev/null; then
        info "Installing ffmpeg…"
        if apt-get install -y ffmpeg &>/dev/null \
            || { apt-get update &>/dev/null && apt-get install -y ffmpeg &>/dev/null; }; then
            success "ffmpeg installed."
            return 0
        fi
        warn "Automatic ffmpeg install failed."
    fi
    warn "Install ffmpeg, then restart the service:"
    warn "  sudo apt-get install -y ffmpeg && sudo systemctl restart birdnet-behavior"
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
