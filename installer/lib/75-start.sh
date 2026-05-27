# ---------------------------------------------------------------------------
# Start the service when appropriate
# ---------------------------------------------------------------------------

maybe_start_service() {
    # Upgrade path: if we stopped a running service to swap the binary, bring
    # it back on the new version. Schema migrations run automatically on
    # startup, and the SQLite/DuckDB data + config were left untouched.
    if [ "${SERVICE_WAS_RUNNING}" = "1" ]; then
        info "Restarting service on the upgraded binary…"
        systemctl start birdnet-behavior.service
        success "Service restarted (schema migrations applied on startup)."
        return
    fi

    # Fresh install: only start if an audio source was written into the config.
    if grep -qE '^(ALSA_CARD|RTSP_URL)=' "${CONFIG_FILE}" 2>/dev/null; then
        info "Audio source detected in config — starting service now…"
        systemctl start birdnet-behavior.service
        success "Service started."
    else
        warn "No audio source configured yet."
        warn "Edit ${CONFIG_FILE}, then: sudo systemctl start birdnet-behavior"
    fi
}
