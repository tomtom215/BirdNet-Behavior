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
