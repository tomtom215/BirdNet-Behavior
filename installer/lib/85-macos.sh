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
