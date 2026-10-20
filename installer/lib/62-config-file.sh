# ---------------------------------------------------------------------------
# Write the default configuration file
# ---------------------------------------------------------------------------

# The geomodel, downloaded on an update or a repair, is used only once the
# config names it — and write_config keeps an existing config. A station
# installed before the geomodel shipped was told "occurrence filtering is ON"
# and ran without it. Fill the settings in where that is certainly wanted:
#   * no METADATA_MODEL_PATH line at all — a config from before the geomodel;
#   * the template's own empty placeholders, written when a fresh install's
#     download failed.
# A line commented out with a path in it is the operator's choice; say so and
# leave it.
enable_geomodel_in_kept_config() {
    [ "${GEOMODEL_INSTALLED:-0}" = "1" ] || return 0
    # The geomodel belongs to the V3.0 classifier. A config that names the V2.4
    # file this installer manages is paired with the range filter instead
    # (enable_range_filter_in_kept_config); writing the geomodel into it would
    # hand one network the other's occurrence filter. Nothing in the install
    # flow gets here with both, so this is the file-level backstop for the
    # invariant rather than a path anyone takes.
    if [ "$(config_value MODEL_PATH "${CONFIG_FILE}")" = "${MODEL_DIR}/${V24_MODEL_FILE}" ]; then
        warn "${CONFIG_FILE} runs BirdNET V2.4, whose occurrence filter is the range filter, not the geomodel — leaving METADATA_* alone."
        return 0
    fi
    local model_line="METADATA_MODEL_PATH=${MODEL_DIR}/${GEOMODEL_FILE}"
    local labels_line="METADATA_LABELS_PATH=${MODEL_DIR}/${GEOMODEL_LABELS_FILE}"
    if ! grep -q 'METADATA_MODEL_PATH' "${CONFIG_FILE}"; then
        printf '\n# --- Species occurrence filtering (added by install.sh) ---\n%s\n%s\n' \
            "${model_line}" "${labels_line}" >>"${CONFIG_FILE}"
        success "Species occurrence filtering is ON: added the geomodel to ${CONFIG_FILE}."
        # A config with no METADATA line was written before the geomodel
        # shipped, so the binary kept as .prev predates it too. 0.15.0 loads
        # this 12K-output geomodel but pairs its outputs with the 11K
        # classifier by position, and with coordinates set it filtered a
        # magpie recording down to no detections. The plain `mv` rollback
        # brought back a station that ran and recorded nothing; this one also
        # takes back the three lines just added (the blank line before the
        # marker stays). Printed after the binary's own rollback line, so it is
        # the last one shown.
        local prev="${INSTALL_DIR}/${BINARY_NAME}.prev"
        if [ -f "${prev}" ]; then
            info "The previous binary predates the geomodel, so roll back with: sudo sed -i '/^# --- Species occurrence filtering (added by install\\.sh) ---\$/,+2d' ${CONFIG_FILE} && sudo mv ${prev} ${INSTALL_DIR}/${BINARY_NAME} && sudo systemctl restart ${SERVICE_NAME}"
        fi
    elif grep -qE '^# METADATA_MODEL_PATH=$' "${CONFIG_FILE}"; then
        sed -i -e "s|^# METADATA_MODEL_PATH=\$|${model_line}|" \
            -e "s|^# METADATA_LABELS_PATH=\$|${labels_line}|" "${CONFIG_FILE}"
        success "Species occurrence filtering is ON: filled in the geomodel settings in ${CONFIG_FILE}."
    elif grep -qE '^[[:space:]]*METADATA_MODEL_PATH=' "${CONFIG_FILE}"; then
        success "Species occurrence filtering is configured in ${CONFIG_FILE}."
    else
        warn "The geomodel is installed, but METADATA_MODEL_PATH is commented out in"
        warn "${CONFIG_FILE}, so species occurrence filtering stays OFF. Uncomment it to use it."
    fi
}

# The V2.4 twin of the function above, for a config that names the V2.4 file.
# Narrower on purpose: it fills in the template's own empty placeholders (the
# state a fresh install's failed range-filter download leaves behind) and
# nothing else. A config with no METADATA_MODEL_PATH line at all was written by
# hand — the installer has never written one without — and switching occurrence
# filtering on in it is the operator's decision, not an update's. The V3.0
# geomodel retrofit adds one because it was introduced as on-by-default in the
# same release; there is no such release to honour here.
enable_range_filter_in_kept_config() {
    [ "${RANGE_FILTER_INSTALLED:-0}" = "1" ] || return 0
    # Only the V2.4 file pairs with the V2.4 range filter. This guard is the
    # other half of the one above: a V3.0 (or any other) config is never given
    # a filter that is indexed like a different network's labels.
    if [ "$(config_value MODEL_PATH "${CONFIG_FILE}")" != "${MODEL_DIR}/${V24_MODEL_FILE}" ]; then
        warn "${CONFIG_FILE} does not run BirdNET V2.4, so the V2.4 range filter was not written into it."
        return 0
    fi
    local model_line="METADATA_MODEL_PATH=${MODEL_DIR}/${V24_RANGE_FILE}"
    if grep -qE '^# METADATA_MODEL_PATH=$' "${CONFIG_FILE}"; then
        sed -i -e "s|^# METADATA_MODEL_PATH=\$|${model_line}|" "${CONFIG_FILE}"
        success "Species occurrence filtering is ON: filled in the range filter in ${CONFIG_FILE}."
    elif grep -qE '^[[:space:]]*METADATA_MODEL_PATH=' "${CONFIG_FILE}"; then
        success "Species occurrence filtering is configured in ${CONFIG_FILE}."
    else
        warn "The range filter is installed, but ${CONFIG_FILE} does not name it, so species"
        warn "occurrence filtering stays OFF. Add this line (and LATITUDE/LONGITUDE) to use it:"
        warn "  ${model_line}"
    fi
}

write_config() {
    if [ -f "${CONFIG_FILE}" ]; then
        warn "Config file already exists at ${CONFIG_FILE} — skipping."
        enable_geomodel_in_kept_config
        enable_range_filter_in_kept_config
        # Upgrade from a version that left the config world-readable: tighten it
        # without touching the user's settings.
        chown "root:${SERVICE_USER}" "${CONFIG_FILE}" 2>/dev/null || true
        chmod 0640 "${CONFIG_FILE}"
        return
    fi

    # Bake interactive / auto-detected answers in; otherwise keep the commented
    # examples. Heredoc expansion is single-pass, so a value containing $ or
    # backticks (e.g. an RTSP URL with credentials) is written literally.
    local alsa_line="# ALSA_CARD=plughw:1,0"
    local rtsp_line="# RTSP_URL=rtsp://camera.local:554/stream"
    local lat_line="# LATITUDE=51.5074"
    local lon_line="# LONGITUDE=-0.1278"
    [ -n "${ALSA_CARD_VALUE}" ] && alsa_line="ALSA_CARD=${ALSA_CARD_VALUE}"
    [ -n "${RTSP_URL_VALUE}" ]  && rtsp_line="RTSP_URL=${RTSP_URL_VALUE}"
    [ -n "${LATITUDE_VALUE}" ]  && lat_line="LATITUDE=${LATITUDE_VALUE}"
    [ -n "${LONGITUDE_VALUE}" ] && lon_line="LONGITUDE=${LONGITUDE_VALUE}"
    local caddy_user_line="# CADDY_USER=admin"
    local caddy_pwd_line="# CADDY_PWD=change-me-to-a-strong-password"
    [ -n "${CADDY_USER_VALUE}" ] && caddy_user_line="CADDY_USER=${CADDY_USER_VALUE}"
    [ -n "${CADDY_PWD_VALUE}" ]  && caddy_pwd_line="CADDY_PWD=${CADDY_PWD_VALUE}"
    # Persist the bind address so `install.sh repair`/`update` keep it (the
    # installer reads BIRDNET_LISTEN back from here on re-run).
    local listen_line="BIRDNET_LISTEN=${LISTEN_ADDR}"
    # The geomodel is optional and its download is non-fatal, so the two
    # settings are only written live when both files actually landed. Writing
    # them unconditionally would point a fresh station at paths that do not
    # exist, and `--doctor` would then report FAIL on an install that had merely
    # declined an optional download.
    #
    # Which filter belongs to the station is decided by its classifier, never by
    # what happens to be on disk: V2.4 gets the range filter (and no labels
    # line — it is indexed like the V2.4 labels), everything else the geomodel
    # and its labels. The two are not interchangeable (see 10-config.sh).
    local geo_model_line="# METADATA_MODEL_PATH="
    local geo_labels_line="# METADATA_LABELS_PATH="
    local filter_lines occurrence_prose detection_prose
    if [ "${CLASSIFIER:-}" = "${CLASSIFIER_V24}" ]; then
        if [ "${RANGE_FILTER_INSTALLED:-0}" = "1" ]; then
            geo_model_line="METADATA_MODEL_PATH=${MODEL_DIR}/${V24_RANGE_FILE}"
        fi
        filter_lines="${geo_model_line}"
        occurrence_prose=$'# Drops species that do not occur near this station at this time of year. Needs
# both of the following; the installer fetches the second for you, and the
# setting below is live when it succeeded, commented out when it did not.
#   1. station coordinates (set above, or on the dashboard)
#   2. METADATA_MODEL_PATH   — the BirdNET V2.4 range filter v2 (ONNX)
# The range filter is indexed like the V2.4 labels, so it needs no
# METADATA_LABELS_PATH of its own; and it means nothing to any other network,
# so do not point another classifier at it, or this one at the V3.0 geomodel.
# Run `birdnet-behavior --doctor` to see which of the two is missing.'
        detection_prose=$'# BirdNET-Pi\'s defaults, which BirdNET V2.4 shares with it and which mean here what
# they mean there. They are shown commented because they are what you get when
# you set nothing.
# CONFIDENCE=0.70          # 0.0–1.0 (detections below this are discarded).
#                          # The setup wizard asks for this on first login; note
#                          # that unlike SF_THRESH, 0 does not mean "disabled" —
#                          # it records every window as a detection.
# SENSITIVITY=1.25         # 0.5–1.5; the same number, with the same meaning, as BirdNET-Pi\'s
# OVERLAP=0.0              # seconds of overlap between analysis windows (0.0 = none)
# PRIVACY_RANK=10          # BirdNET-Pi\'s human-sound rule: drop a window (and its neighbours) when a human sound ranks in the model\'s top 10 classes; 0 = off
# DATABASE_LANG=en'
    else
        if [ "${GEOMODEL_INSTALLED:-0}" = "1" ]; then
            geo_model_line="METADATA_MODEL_PATH=${MODEL_DIR}/${GEOMODEL_FILE}"
            geo_labels_line="METADATA_LABELS_PATH=${MODEL_DIR}/${GEOMODEL_LABELS_FILE}"
        fi
        filter_lines="${geo_model_line}"$'\n'"${geo_labels_line}"
        occurrence_prose=$'# Drops species that do not occur near this station at this time of year. Needs
# all three of the following; the installer fetches the last two for you, and
# the two settings below are live when it succeeded, commented out when it did
# not.
#   1. station coordinates (set above, or on the dashboard)
#   2. METADATA_MODEL_PATH   — the BirdNET geomodel (ONNX)
#   3. METADATA_LABELS_PATH  — that model\'s own label file
# The geomodel scores a different species list from the classifier (12 012
# against 11 560), so the label file is what maps one onto the other; omit it
# only for a metadata model indexed identically to the classifier, which the
# station verifies at startup and refuses if it does not hold.
# Run `birdnet-behavior --doctor` to see which of the three is missing.'
        detection_prose=$'# CONFIDENCE=0.70          # 0.0–1.0, default 0.70 (detections below this are discarded).
#                          # The setup wizard asks for this on first login; note
#                          # that unlike SF_THRESH, 0 does not mean "disabled" —
#                          # it records every window as a detection.
# SENSITIVITY=1.25         # 0.5–1.5, default 1.25 (V2.4 models only; V3.0 ignores it)
# OVERLAP=0.0              # seconds of overlap between analysis windows
# PRIVACY_RANK=10          # BirdNET-Pi\'s human-sound rule: drop a window (and its neighbours) when a human sound ranks in the model\'s top 10 classes; 0 = off
# DATABASE_LANG=en'
    fi

    info "Writing default config to ${CONFIG_FILE}…"
    cat > "${CONFIG_FILE}" <<EOF
# BirdNet-Behavior configuration
# Generated by install.sh on $(date -u +"%Y-%m-%d %H:%M UTC")
#
# To change a setting safely, edit a copy and apply it — the copy is
# validated first, the previous file is kept as a backup, and the service is
# restarted:
#   sudo cp ${CONFIG_FILE} /tmp/birdnet.conf && sudo nano /tmp/birdnet.conf
#   sudo ${INSTALL_DIR}/${BINARY_NAME} --apply-config /tmp/birdnet.conf
# Editing this file in place and restarting also works; a file with errors
# is then run around, not on: the station starts on the last configuration a
# start succeeded on (${CONFIG_FILE}.last-good) and says so at /station.

# --- Paths ---
DB_PATH=${DB_PATH}
RECS_DIR=${RECS_DIR}
IMAGE_CACHE_DIR=${IMAGE_CACHE_DIR}

# --- Model (${CLASSIFIER_NAME:-the classifier}, downloaded automatically by installer) ---
MODEL_PATH=${MODEL_DIR}/${MODEL_FILE}
LABELS_PATH=${MODEL_DIR}/${LABELS_FILE}

# --- Audio source ---
# Use one of: ALSA microphone, RTSP stream, or an existing recordings directory.
${alsa_line}
${rtsp_line}
# Multiple RTSP cameras: comma-separated. Each becomes its own capture stream
# (filenames/metrics prefixed RTSP_1, RTSP_2, …). Overrides RTSP_URL when set.
# RTSP_URLS=rtsp://cam1:554/stream,rtsp://cam2:554/stream

# --- Location (used for species frequency filtering and BirdWeather) ---
${lat_line}
${lon_line}
# RECORDING_SCHEDULE=all-day   # all-day | solar | fixed:HH:MM-HH:MM
#                              # solar records sunrise-to-sunset and needs the
#                              # coordinates above. Fixed hours are evaluated in
#                              # UTC, not local time. Also settable in the web UI
#                              # under Settings -> Capture -> Location & Recording Schedule.

# --- Detection ---
${detection_prose}

# EXTRACTION_LENGTH=6      # seconds of audio saved around each detection
# RARE_SPECIES_THRESHOLD   # set on /admin/settings (days since last heard)

# --- Logging ---
# LOG_LEVEL=info           # trace|debug|info|warn|error|off (RUST_LOG wins)
# LOG_MODULES=             # per subsystem, e.g. audio=debug,web=warn
#                          # names: audio detection web db integrations analytics

# --- Species occurrence filtering ---
${occurrence_prose}
${filter_lines}
# SF_THRESH=0.03           # occurrence threshold; no effect while the filter is off

# --- Disk management ---
# MAX_FILES_SPECIES=0      # 0 = keep all recordings per species; set e.g. 100 to cap
# DISK_PURGE_THRESHOLD=95
# Raw capture segments land in the RAM-backed stream dir (--watch-dir, default
# /tmp/birdnet-stream) and are drained once the detector has processed them, so
# the tmpfs cannot fill. Tune the rolling buffer if needed:
# STREAM_RETENTION_SECS=600  # delete processed raw segments older than this (0 = off)
# STREAM_MAX_MB=512          # hard cap on the stream dir; oldest segments drop first (0 = off)

# --- Notifications (Apprise) ---
# APPRISE_URL=http://localhost:8000

# --- BirdWeather ---
# BIRDWEATHER_TOKEN=your-token-here

# --- Site name shown in web UI ---
# SITENAME=My Bird Station

# --- Web dashboard bind address ---
# Default 0.0.0.0:8502 = reachable from other devices on your LAN. Viewing the
# dashboard is open; the /admin panel (settings, software update) is gated by
# CADDY_PWD below. To restrict the WHOLE dashboard to this device, set
# 127.0.0.1:8502, then apply it with:  sudo bash install.sh repair
${listen_line}

# --- Admin authentication (CADDY_PWD) ---
# The /admin panel can change settings, trigger backups, and update the
# software, so it requires a sign-in through the dashboard's login form.
# A fresh install sets a strong CADDY_PWD automatically; change it here any
# time, then apply it with:  sudo bash install.sh repair
#
# Sign in with the username "admin" — that is the account the dashboard seeds,
# and it is the only one that exists until you create more in the admin panel.
# CADDY_USER below does NOT rename it: the login form reads CADDY_USER from the
# process environment only, and this unit sets no EnvironmentFile, so on a
# bare-metal install it has no effect. It is honoured under Docker, where the
# environment does reach the process.
#
# Clearing CADDY_PWD leaves /admin OPEN to anyone who can reach the dashboard.
${caddy_user_line}
${caddy_pwd_line}
EOF
    # The config can hold secrets (CADDY_PWD, BIRDWEATHER_TOKEN), so keep it
    # readable by the service's group but never world-readable. Root owns it so
    # the non-root service can read but not rewrite its own config.
    chown "root:${SERVICE_USER}" "${CONFIG_FILE}"
    chmod 0640 "${CONFIG_FILE}"
    success "Default config written — edit ${CONFIG_FILE} to configure your station."
    if [ "${GEOMODEL_INSTALLED:-0}" = "1" ]; then
        success "Species occurrence filtering is ON (threshold SF_THRESH, default 0.03)."
    fi
}
