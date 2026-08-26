# ---------------------------------------------------------------------------
# Start the service when appropriate
# ---------------------------------------------------------------------------

# True (0) if the config has an *active* (uncommented) audio source — ALSA_CARD,
# RTSP_URL(S), or PIPEWIRE_DEVICE. Used only to tailor the post-install message;
# the service is started regardless (a station with no source still serves the
# dashboard + onboarding wizard, exactly as it does on a reboot).
config_has_audio_source() {
    grep -qE '^[[:space:]]*(ALSA_CARD|RTSP_URL|RTSP_URLS|PIPEWIRE_DEVICE)[[:space:]]*=' \
        "${CONFIG_FILE}" 2>/dev/null
}

# True (0) if the config has *both* an active LATITUDE and LONGITUDE.
#
# Same "tailor the message" role as config_has_audio_source, and the same
# reason for existing: without coordinates the metadata model cannot run, so
# occurrence filtering is skipped and every species in the model stays a
# candidate. The station looks like it is working — it just reports birds from
# the wrong continent, which reads as a bad model rather than a missing
# setting. A non-interactive install (`BIRDNET_NONINTERACTIVE=1`, or no TTY
# under `curl | sudo bash`) never reaches the location prompt at all, so this
# is the common state, not the rare one.
#
# Both halves are required: `birdnet_core::config::validate` already warns
# about one without the other, and one alone disables the filter just as
# completely as neither.
config_has_location() {
    grep -qE '^[[:space:]]*LATITUDE[[:space:]]*=[[:space:]]*[^[:space:]]' \
        "${CONFIG_FILE}" 2>/dev/null \
    && grep -qE '^[[:space:]]*LONGITUDE[[:space:]]*=[[:space:]]*[^[:space:]]' \
        "${CONFIG_FILE}" 2>/dev/null
}

maybe_start_service() {
    # No systemd here (container / chroot / staged install): the unit is on disk
    # but there is nothing to start it. install_service already told the operator
    # how to finish on a real host; nothing more to do.
    if ! has_systemd; then
        return
    fi

    # Upgrade path: if we stopped a running service to swap the binary, bring
    # it back on the new version. Schema migrations run automatically on
    # startup, and the SQLite/DuckDB data + config were left untouched.
    if [ "${SERVICE_WAS_RUNNING}" = "1" ]; then
        info "Restarting service on the upgraded binary…"
        # Cleared first: restore_service_if_we_stopped_it is armed as an EXIT
        # trap and must not start it a second time.
        SERVICE_WAS_RUNNING=0
        systemctl start birdnet-behavior.service
        success "Service restarted (schema migrations applied on startup)."
        return
    fi

    # Fresh install: start the service now so the dashboard comes up immediately.
    # The unit is enabled (see install_service), so systemd brings it up on the
    # next reboot no matter what — starting it here closes the confusing gap
    # where nothing appears after install but a reboot "fixes" it.
    #
    # An audio source is deliberately NOT required to start. The web dashboard —
    # and its first-run onboarding wizard, where the operator picks a microphone
    # and sets their location — is the whole point of a fresh install, and the
    # detection daemon idles harmlessly until a source exists. The unit's doctor
    # preflight treats "no audio source" as a warning, not a failure, so the
    # start succeeds either way. Mirrors the Docker quickstart, which has always
    # brought the dashboard up regardless of audio.
    info "Starting service now…"
    if systemctl start birdnet-behavior.service; then
        if config_has_audio_source; then
            success "Service started."
        else
            success "Service started — finish setup in the dashboard."
            info  "No audio source yet: pick a microphone in the dashboard's setup wizard"
            info  "(or set ALSA_CARD / RTSP_URL in ${CONFIG_FILE}); detection begins once one is set."
        fi
    else
        warn "Service failed to start — inspect: sudo journalctl -u birdnet-behavior -e"
        warn "Once resolved: sudo systemctl start birdnet-behavior"
    fi
}
