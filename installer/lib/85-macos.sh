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

# Put the classifier's files in ${MAC_DATA_DIR}/models by asking the binary to
# install them (`--install-model`), not by downloading them here.
#
# Linux's installer fetches and checks the files itself; macOS does not ship
# `sha256sum`, and the binary already carries the same sha256 pins (the Rust
# catalogue is the source of truth the Linux installer's are tested against),
# fetches atomically through a .part file, and refuses a size or digest it
# cannot verify. The files land under the names the Rust installer gives them
# (birdnet-v24.onnx, birdnet-v24_labels.txt, birdnet-v24-range-v2.onnx; for the
# V3.0 preview birdnet-v3.onnx and birdnet-v3_labels.csv), which is what the
# config below names. The V3.0 geomodel is not in that catalogue, so a macOS
# V3.0 station has no occurrence filter until one is added by hand.
#
# Non-fatal: a failed fetch leaves the config naming the files and prints the
# command that fetches them, so an install on a flaky link can be finished
# later. Returns 1 when any file could not be installed.
macos_install_models() { # $1=binary path  $2=classifier id
    local bin="$1" id="$2" dir="${MAC_DATA_DIR}/models" item rc=0 items
    mkdir -p "${dir}"
    case "${id}" in
        birdnet-v24) items="birdnet-v24 birdnet-v24-range-v2" ;;
        *)           items="birdnet-v3" ;;
    esac
    for item in ${items}; do
        info "Installing ${item} into ${dir} (sha256-verified)…"
        # Without the installer's own BIRDNET_* settings (the binary reports every
        # one it does not read) and with an empty config (there is none yet).
        if ! env -u BIRDNET_CLASSIFIER -u BIRDNET_NONINTERACTIVE -u BIRDNET_BINARY_TARBALL \
            BIRDNET_MODEL_DIR="${dir}" "${bin}" --config /dev/null --install-model "${item}"; then
            warn "Could not install ${item}. Finish later with:"
            warn "  BIRDNET_MODEL_DIR=\"${dir}\" \"${bin}\" --install-model ${item}"
            rc=1
        fi
    done
    return "${rc}"
}

macos_setup_config_and_agent() { # $1=binary path
    local bin="$1"
    mkdir -p "${MAC_DATA_DIR}" "${HOME}/Library/Logs" "$(dirname "${MAC_PLIST}")"
    if [ ! -f "${MAC_DATA_DIR}/birdnet.conf" ]; then
        # A config is written once, so there is no station here to keep: the
        # request, else the default. (An existing config below is left as it is,
        # and so is the model it names.)
        if [ -n "${CLASSIFIER_REQUEST}" ] && ! valid_classifier_id "${CLASSIFIER_REQUEST}"; then
            fatal "Unknown classifier '${CLASSIFIER_REQUEST}'. Choose ${CLASSIFIER_V24} (the default) or ${CLASSIFIER_V3}."
        fi
        local mac_classifier="${CLASSIFIER_REQUEST:-${DEFAULT_CLASSIFIER}}"
        local mac_models="${MAC_DATA_DIR}/models"
        local mac_model_path mac_labels_path mac_filter_line mac_name
        if [ "${mac_classifier}" = "${CLASSIFIER_V24}" ]; then
            mac_name="BirdNET V2.4"
            mac_model_path="${mac_models}/${V24_MODEL_FILE}"
            mac_labels_path="${mac_models}/${V24_LABELS_FILE}"
            mac_filter_line="# METADATA_MODEL_PATH=${mac_models}/${V24_RANGE_FILE}"
        else
            mac_name="BirdNET+ V3.0 preview"
            mac_model_path="${mac_models}/birdnet-v3.onnx"
            mac_labels_path="${mac_models}/birdnet-v3_labels.csv"
            mac_filter_line="# METADATA_MODEL_PATH="
            info "BirdNET+ V3.0 on macOS runs without species-occurrence filtering: the binary does not install the geomodel (the Linux installer does)."
        fi
        if macos_install_models "${bin}" "${mac_classifier}"; then
            # Live only when the file is really there, so a failed fetch cannot
            # leave a config the doctor reports as FAIL.
            [ "${mac_classifier}" = "${CLASSIFIER_V24}" ] && mac_filter_line="${mac_filter_line#\# }"
            success "Classifier: ${mac_name}."
        else
            warn "The config below names the ${mac_name} files; the station detects nothing until they are installed."
        fi
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

# --- Model (${mac_name}; installed by install.sh) ---
MODEL_PATH=${mac_model_path}
LABELS_PATH=${mac_labels_path}
# Species-occurrence filtering needs LATITUDE and LONGITUDE above as well.
${mac_filter_line}
CONF
        success "Wrote starter config: ${MAC_DATA_DIR}/birdnet.conf"
    else
        info "Keeping existing config: ${MAC_DATA_DIR}/birdnet.conf"
    fi
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
    echo "  4. Open http://localhost:8502"
    echo "     The classifier's files are in ${MAC_DATA_DIR}/models; MODEL_PATH in the config names them."
    echo "     New installs take --classifier birdnet-v24 (default) or birdnet-v3; an existing config is kept."
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
        track_tmpdir "${tmp}"
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
