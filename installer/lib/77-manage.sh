# ---------------------------------------------------------------------------
# Top-level flows: install / update / reinstall / repair / uninstall, and the
# interactive menu shown when an existing install is detected.
# ---------------------------------------------------------------------------

# Stop a running service before swapping the binary. You cannot overwrite a
# running executable in place (ETXTBSY), and a plain `systemctl start` on an
# already-running unit would not load the new binary. Records that it was
# running so the service is restarted afterwards.
stop_running_service_for_swap() {
    if systemctl is-active --quiet "${SERVICE_NAME}" 2>/dev/null; then
        SERVICE_WAS_RUNNING=1
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

    stop_running_service_for_swap

    install_binary "${version}" "${arch}"
    create_directories
    setup_tmpfs_streaming
    download_model
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
    systemctl is-active --quiet "${SERVICE_NAME}" 2>/dev/null && was_active=1

    # Binary: only (re)download if it is actually missing.
    if [ ! -x "${INSTALL_DIR}/${BINARY_NAME}" ]; then
        warn "Binary missing — downloading it."
        local arch version
        arch="$(detect_arch)"
        check_glibc
        version="$(resolve_version)"
        stop_running_service_for_swap
        install_binary "${version}" "${arch}"
    else
        success "Binary present ($("${INSTALL_DIR}/${BINARY_NAME}" --version 2>/dev/null | head -1))."
    fi

    create_directories
    setup_tmpfs_streaming

    if [ -f "${MODEL_DIR}/${MODEL_FILE}" ] && [ -f "${MODEL_DIR}/${LABELS_FILE}" ]; then
        success "Model present — skipping download."
    else
        warn "Model files missing — downloading."
        download_model
    fi

    write_config             # idempotent: fixes ownership/permissions, keeps content
    resolve_listen_addr      # preserve LAN bind across re-runs (env > config > unit)
    ensure_capture_backend   # RTSP stations need ffmpeg or the daemon can't start
    install_service          # rewrites the unit (this is what fixes a bad unit) + reload

    # Clear any failed / rate-limited state from prior crash loops, then bring
    # the service up with the repaired unit.
    systemctl reset-failed "${SERVICE_NAME}" 2>/dev/null || true
    if [ "${was_active}" = 1 ] || grep -qE '^(ALSA_CARD|RTSP_URL)=' "${CONFIG_FILE}" 2>/dev/null; then
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
    if command -v systemctl >/dev/null 2>&1 \
        && systemctl list-unit-files 2>/dev/null | grep -q "^${SERVICE_NAME}"; then
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
