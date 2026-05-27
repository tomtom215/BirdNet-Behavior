# ---------------------------------------------------------------------------
# Detect and configure the audio device + station location
# ---------------------------------------------------------------------------

# Returns the first detected ALSA capture device as "plughw:<card>,<device>",
# or an empty string if none found / arecord not available.
detect_first_audio_device() {
    command -v arecord &>/dev/null || return 0
    # arecord -l output looks like: card 1: Device [USB Audio Device], device 0: ...
    local first_card first_device
    first_card="$(arecord -l 2>/dev/null | awk '/^card/{print $2; exit}' | tr -d ':')"
    # 2-arg match() (RSTART/RLENGTH) is POSIX; the 3-arg capture form is a gawk
    # extension that errors on mawk (the default awk on Debian / Raspberry Pi OS).
    first_device="$(arecord -l 2>/dev/null | awk '/^card/{ if (match($0, /device [0-9]+/)) print substr($0, RSTART + 7, RLENGTH - 7); exit }')"
    if [ -n "${first_card}" ]; then
        echo "plughw:${first_card},${first_device:-0}"
    fi
}

# True (0) if v is a decimal number within [lo, hi]. Used to sanity-check
# latitude/longitude so a typo doesn't get written into the config.
valid_coord() {
    awk -v v="$1" -v lo="$2" -v hi="$3" \
        'BEGIN { if (v ~ /^[-+]?([0-9]+\.?[0-9]*|\.[0-9]+)$/ && v+0 >= lo && v+0 <= hi) exit 0; exit 1 }'
}

# Collect the audio source and station location on a fresh install. When
# interactive we ask the operator directly (so a non-technical user gets a
# working station without hand-editing a file); otherwise we keep the historical
# behaviour — silently auto-detect ALSA and leave location for the config file.
# The config file always remains editable and is the source of truth afterwards.
prompt_station_settings() {
    # Re-install / upgrade: the config already exists and is the user's own —
    # never re-prompt or overwrite their settings.
    [ -f "${CONFIG_FILE}" ] && return 0

    local candidate
    candidate="$(detect_first_audio_device)"

    if [ "${INTERACTIVE}" != "1" ]; then
        if [ -n "${candidate}" ]; then
            ALSA_CARD_VALUE="${candidate}"
            info "Auto-detected ALSA device: ${candidate}"
        else
            warn "No ALSA device detected — set ALSA_CARD or RTSP_URL in ${CONFIG_FILE}."
        fi
        return 0
    fi

    # ---- Audio source ----
    printf '\n  Audio source\n' >/dev/tty
    if [ -n "${candidate}" ]; then
        arecord -l 2>/dev/null | grep '^card' | sed 's/^/    /' >/dev/tty || true
        yesno "  Use detected ALSA device '${candidate}'?" y && ALSA_CARD_VALUE="${candidate}"
    else
        printf '  No ALSA capture device detected.\n' >/dev/tty
    fi
    if [ -z "${ALSA_CARD_VALUE}" ]; then
        local audio_in
        audio_in="$(ask "  Audio source — ALSA device (e.g. plughw:1,0) or rtsp:// URL (Enter to skip)" "")"
        case "${audio_in}" in
            '')                   : ;;
            rtsp://* | rtsps://*) RTSP_URL_VALUE="${audio_in}" ;;
            *)                    ALSA_CARD_VALUE="${audio_in}" ;;
        esac
    fi
    if [ -n "${ALSA_CARD_VALUE}" ]; then
        success "Audio source: ALSA ${ALSA_CARD_VALUE}"
    elif [ -n "${RTSP_URL_VALUE}" ]; then
        success "Audio source: RTSP ${RTSP_URL_VALUE}"
    else
        warn "No audio source set — add ALSA_CARD or RTSP_URL to ${CONFIG_FILE} later."
    fi

    # ---- Station location ----
    printf '\n  Station location (solar schedule, species filter, BirdWeather)\n' >/dev/tty
    printf '  Tip: right-click your spot on https://openstreetmap.org and read off the coordinates.\n' >/dev/tty
    local lat lon
    lat="$(ask "  Latitude  (e.g. 42.3601, Enter to skip)" "")"
    if [ -n "${lat}" ]; then
        lon="$(ask "  Longitude (e.g. -71.0589)" "")"
        if valid_coord "${lat}" -90 90 && valid_coord "${lon}" -180 180; then
            LATITUDE_VALUE="${lat}"
            LONGITUDE_VALUE="${lon}"
            success "Location: ${lat}, ${lon}"
        else
            warn "Coordinates '${lat}, ${lon}' look invalid — skipping; set LATITUDE/LONGITUDE in ${CONFIG_FILE} later."
        fi
    fi

    # ---- Web dashboard exposure ----
    printf '\n  Web dashboard\n' >/dev/tty
    printf '  By default the dashboard is reachable only from this device (localhost).\n' >/dev/tty
    if yesno "  Make it reachable from other devices on your network?" n; then
        printf '  Its admin can change settings and update software, so protect it.\n' >/dev/tty
        local pw1 pw2
        pw1="$(ask_secret "  Set a dashboard password (Enter to skip)")"
        if [ -n "${pw1}" ]; then
            pw2="$(ask_secret "  Confirm password")"
            if [ "${pw1}" = "${pw2}" ]; then
                CADDY_USER_VALUE="birdnet"
                CADDY_PWD_VALUE="${pw1}"
                LISTEN_ADDR="0.0.0.0:8502"
                success "Dashboard on the LAN, password-protected (username: birdnet)."
            else
                warn "Passwords did not match — keeping the dashboard on localhost only."
            fi
        elif yesno "  Expose to the LAN with NO password?" n; then
            LISTEN_ADDR="0.0.0.0:8502"
            warn "Dashboard on the LAN with NO authentication — anyone on the network can change settings. Add CADDY_PWD to ${CONFIG_FILE} to fix this."
        else
            success "Keeping the dashboard on localhost only."
        fi
    else
        success "Dashboard stays on this device (localhost) — SSH-tunnel in, or set BIRDNET_LISTEN to expose it."
    fi
}
