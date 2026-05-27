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
    write_config
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

    write_config       # idempotent: fixes ownership/permissions, keeps content
    install_service    # rewrites the unit (this is what fixes a bad unit) + reload

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

# Software-only uninstall (data preserved). For removing data/model too, point
# the operator at the dedicated uninstall.sh, which has the data flags.
do_uninstall() {
    info "Stopping and removing BirdNet-Behavior (data preserved)…"
    systemctl stop "${SERVICE_NAME}" 2>/dev/null || true
    systemctl disable "${SERVICE_NAME}" 2>/dev/null || true
    rm -f "${SERVICE_FILE}"
    # Tear down the tmpfs mount unit install may have added (escaped '-' = \x2d).
    systemctl disable --now 'tmp-birdnet\x2dstream.mount' 2>/dev/null || true
    rm -f '/etc/systemd/system/tmp-birdnet\x2dstream.mount'
    systemctl daemon-reload 2>/dev/null || true
    rm -f "${INSTALL_DIR}/${BINARY_NAME}"
    rm -rf "${STREAM_DIR}"
    success "Binary, service, and tmpfs unit removed."
    warn "Data and config preserved at ${DATA_DIR} and ${CONFIG_FILE}."
    warn "To remove the database, recordings, and model too:  sudo ./uninstall.sh --purge"
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
