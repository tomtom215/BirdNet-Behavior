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

# Put the classifier's files, and its occurrence filter's, in
# ${MAC_DATA_DIR}/models by asking the binary to install them (`--install-model`)
# — the one install mechanism every installer uses (55-model.sh says what it
# does). The files land under the names the binary gives them
# (birdnet-v24.onnx, birdnet-v24_labels.txt, birdnet-v24-range-v2.onnx; for the
# V3.0 preview birdnet-v3.onnx, birdnet-v3_labels.csv, birdnet-v3-geomodel.onnx
# and birdnet-v3-geomodel_labels.txt), which is what the config below names.
#
# Run on every install, also over a config that is kept, so a re-run retries
# what failed and replaces what does not verify. A failed classifier is
# recorded (CLASSIFIER_MISSING) and said loudly at the end, and the run exits
# EXIT_NO_CLASSIFIER, as on Linux; a failed filter only warns.
#
# Sets MAC_FILTER_INSTALLED=1 when the occurrence filter is in place.
MAC_FILTER_INSTALLED=0
macos_install_models() { # $1=binary path  $2=classifier id
    local classifier="$2" filter
    MODEL_BINARY="$1"
    MODEL_DIR="${MAC_DATA_DIR}/models"
    mkdir -p "${MODEL_DIR}"
    select_classifier "${classifier}"
    filter="${FILTER_ID}"
    info "Installing ${CLASSIFIER_NAME} into ${MODEL_DIR} (sha256-verified)…"
    if [ -n "${LOCAL_MODEL}" ]; then
        install_model_with_binary "${classifier}" --from-file "${LOCAL_MODEL}" \
            || classifier_not_installed "${LOCAL_MODEL} was refused: it is not the ${CLASSIFIER_NAME} model this release pins (the reason is above)"
    elif ! install_model_with_binary "${classifier}"; then
        classifier_not_installed "${CLASSIFIER_NAME} could not be downloaded and verified ($(model_attempts) attempts)"
    fi
    info "Installing its species-occurrence filter (${filter})…"
    if install_model_with_binary "${filter}"; then
        MAC_FILTER_INSTALLED=1
    else
        warn "The occurrence filter could not be installed; the station runs without it."
        warn "Run this installer again to retry."
    fi
}

# The classifier a kept macOS config runs: one of the two whose files this
# installer manages, by the MODEL_PATH it names, or nothing (a model of the
# operator's own, which is theirs to install).
macos_kept_classifier() { # $1=config file
    local models="${MAC_DATA_DIR}/models" model
    model="$(config_value MODEL_PATH "$1")"
    case "${model}" in
        "${models}/${V24_MODEL_FILE}") echo "${CLASSIFIER_V24}" ;;
        "${models}/${V3_MODEL_FILE}") echo "${CLASSIFIER_V3}" ;;
        *) ;;
    esac
}

# Fill in a kept macOS config's commented-out filter lines — the ones this
# installer wrote when the filter was not there — now that it is (MDL-14).
macos_enable_filter_in_kept_config() { # $1=config file  $2=classifier id
    local conf="$1" models="${MAC_DATA_DIR}/models"
    [ "${MAC_FILTER_INSTALLED}" = 1 ] || return 0
    if [ "$2" = "${CLASSIFIER_V24}" ]; then
        sed -i.bak -e "s|^# METADATA_MODEL_PATH=\(${models}/${V24_RANGE_FILE}\)\{0,1\}\$|METADATA_MODEL_PATH=${models}/${V24_RANGE_FILE}|" "${conf}"
    else
        # A config the previous macOS installer wrote for V3.0 has the model's
        # placeholder and no labels line at all; the geomodel is unusable
        # without its labels, so the line is added beside the model's.
        awk -v m="METADATA_MODEL_PATH=${models}/${GEOMODEL_FILE}" \
            -v l="METADATA_LABELS_PATH=${models}/${GEOMODEL_LABELS_FILE}" '
            { lines[NR] = $0 }
            /^#? ?METADATA_LABELS_PATH=/ { has_labels = 1 }
            END {
                for (i = 1; i <= NR; i++) {
                    if (lines[i] == "# METADATA_MODEL_PATH=") {
                        print m
                        if (!has_labels) print l
                    } else if (lines[i] == "# METADATA_LABELS_PATH=") {
                        print l
                    } else {
                        print lines[i]
                    }
                }
            }' "${conf}" >"${conf}.new" && mv "${conf}.new" "${conf}"
    fi
    rm -f "${conf}.bak"
    if grep -qE '^METADATA_MODEL_PATH=' "${conf}"; then
        success "Species-occurrence filtering is configured in ${conf}."
    fi
}

# The last thing a macOS run says when the classifier is not installed: the
# Linux summary's words, with the macOS commands.
macos_print_classifier_missing() { # $1=binary path
    [ "${CLASSIFIER_MISSING}" = 1 ] || return 0
    echo
    loud_warn "THIS STATION IS NOT DETECTING BIRDS: its classifier is not installed." \
              "  Why: ${CLASSIFIER_MISSING_WHY}." \
              "  Everything else is installed. To install the classifier, once this Mac" \
              "  can reach the internet, run this installer again (it keeps your config):" \
              "      bash install.sh" \
              "  or, from a copy of the model file you already have (it must match the" \
              "  same pinned sha256 as a download, or it is refused):" \
              "      bash install.sh --local-model /path/to/that/file" \
              "  Then check with:  \"$1\" --doctor --config \"${MAC_DATA_DIR}/birdnet.conf\"" \
              "  This run exits with status ${EXIT_NO_CLASSIFIER}: installed, not detecting."
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
        local mac_model_path mac_labels_path mac_filter_lines mac_name
        macos_install_models "${bin}" "${mac_classifier}"
        mac_name="${CLASSIFIER_NAME}"
        mac_model_path="${mac_models}/${MODEL_FILE}"
        mac_labels_path="${mac_models}/${LABELS_FILE}"
        # Live only when the filter is really there, so a failed fetch cannot
        # leave a config the doctor reports as FAIL.
        if [ "${mac_classifier}" = "${CLASSIFIER_V24}" ]; then
            mac_filter_lines="# METADATA_MODEL_PATH=${mac_models}/${V24_RANGE_FILE}"
            [ "${MAC_FILTER_INSTALLED}" = 1 ] && mac_filter_lines="METADATA_MODEL_PATH=${mac_models}/${V24_RANGE_FILE}"
        elif [ "${MAC_FILTER_INSTALLED}" = 1 ]; then
            mac_filter_lines="METADATA_MODEL_PATH=${mac_models}/${GEOMODEL_FILE}"$'\n'"METADATA_LABELS_PATH=${mac_models}/${GEOMODEL_LABELS_FILE}"
        else
            mac_filter_lines="# METADATA_MODEL_PATH="$'\n'"# METADATA_LABELS_PATH="
        fi
        if [ "${CLASSIFIER_MISSING}" = 1 ]; then
            warn "The config below names the ${mac_name} files; the station detects nothing until they are installed."
        else
            success "Classifier: ${mac_name}."
        fi
        cat > "${MAC_DATA_DIR}/birdnet.conf" <<CONF
# BirdNet-Behavior config (macOS). Set your coordinates and a mic device.
#
# LATITUDE/LONGITUDE are left commented deliberately. They used to be written
# as 0.0/0.0, which is not "unset" — it is Null Island in the Gulf of Guinea,
# and the metadata model would filter this station's species list for that
# spot. An unset location skips occurrence filtering entirely, which is honest;
# a wrong one produces a confident species list for the wrong continent, and
# \`config_has_location\` would report the station as configured.
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
${mac_filter_lines}
CONF
        success "Wrote starter config: ${MAC_DATA_DIR}/birdnet.conf"
    else
        info "Keeping existing config: ${MAC_DATA_DIR}/birdnet.conf"
        local kept
        kept="$(macos_kept_classifier "${MAC_DATA_DIR}/birdnet.conf")"
        if [ -n "${kept}" ]; then
            macos_install_models "${bin}" "${kept}"
            macos_enable_filter_in_kept_config "${MAC_DATA_DIR}/birdnet.conf" "${kept}"
        else
            info "Its MODEL_PATH names a model of your own, so no model is installed or changed."
        fi
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
