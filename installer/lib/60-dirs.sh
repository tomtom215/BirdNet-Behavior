# ---------------------------------------------------------------------------
# Create data directories and the tmpfs streaming directory
# ---------------------------------------------------------------------------

create_directories() {
    info "Creating data directories…"
    # Directories owned by the service user, not root.
    install -d -m 0750 -o "${SERVICE_USER}" -g "${SERVICE_USER}" \
        "${DATA_DIR}" \
        "${RECS_DIR}" \
        "${IMAGE_CACHE_DIR}" \
        "${MODEL_DIR}" \
        "${DATA_DIR}/backups"
    install -d -m 0755 "${CONFIG_DIR}"
    success "Directories created under ${DATA_DIR}"
}

setup_tmpfs_streaming() {
    info "Setting up tmpfs for audio streaming (SD card wear protection)…"
    # Use /tmp/birdnet-stream for raw audio capture. On most Pi distros /tmp is
    # already a tmpfs; this ensures the streaming directory exists after reboot.
    #
    # NOTE: under systemd the service runs with PrivateTmp=yes, so it gets its
    # OWN /tmp and recreates /tmp/birdnet-stream there on every start (see the
    # ExecStartPre= in install_service). This host-side directory is what a
    # manual `birdnet-behavior --watch-dir /tmp/birdnet-stream` run (outside
    # systemd) uses, and is harmless under the service.
    install -d -m 0750 -o "${SERVICE_USER}" -g "${SERVICE_USER}" "${STREAM_DIR}"

    # If /tmp is NOT already tmpfs, create a dedicated mount.
    if findmnt -t tmpfs /tmp &>/dev/null; then
        success "/tmp is already tmpfs — ${STREAM_DIR} is RAM-backed"
    elif has_systemd; then
        local MOUNT_UNIT="/etc/systemd/system/tmp-birdnet\\x2dstream.mount"
        cat > "${MOUNT_UNIT}" <<MEOF
[Unit]
Description=tmpfs for BirdNet-Behavior audio streaming
Before=birdnet-behavior.service

[Mount]
What=tmpfs
Where=${STREAM_DIR}
Type=tmpfs
Options=size=64M,mode=0750,uid=$(id -u "${SERVICE_USER}"),gid=$(id -g "${SERVICE_USER}")

[Install]
WantedBy=multi-user.target
MEOF
        systemctl daemon-reload
        systemctl enable --now "tmp-birdnet\\x2dstream.mount" 2>/dev/null || true
        success "tmpfs mount unit installed for ${STREAM_DIR}"
    else
        # No systemd to manage a tmpfs mount; the plain directory created above
        # is enough for a manual / container run (it just isn't RAM-backed).
        success "Streaming directory ${STREAM_DIR} ready (no systemd tmpfs mount)."
    fi
}
