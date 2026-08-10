# ---------------------------------------------------------------------------
# Detect and configure the audio device + station location
# ---------------------------------------------------------------------------

# Returns the first detected ALSA capture device as "plughw:<card>,<device>",
# or an empty string if none found / arecord not available.
detect_first_audio_device() {
    # No arecord means no card list AND no capture backend. check_required_tools
    # tries to install alsa-utils before we get here, so reaching this branch
    # means that failed (or apt-get is not this distro's package manager) —
    # say so, because a silent empty result reads as "no microphone attached"
    # and produces a station that records nothing without ever complaining.
    if ! command -v arecord &>/dev/null; then
        warn "arecord not found — cannot detect a microphone. Install alsa-utils, then:"
        warn "  sudo bash install.sh repair"
        return 0
    fi
    # arecord -l output looks like:
    #   card 1: PRO [Comica_Traxshot PRO], device 0: USB Audio [USB Audio]
    #          ^index ^ALSA card id
    local listing first_card first_id first_device
    listing="$(arecord -l 2>/dev/null)"
    [ -n "${listing}" ] || return 0

    first_card="$(printf '%s\n' "${listing}" | awk '/^card /{ print $2; exit }' | tr -d ':')"
    [ -n "${first_card}" ] || return 0
    first_id="$(printf '%s\n' "${listing}" | awk '/^card /{ print $3; exit }')"
    # 2-arg match() (RSTART/RLENGTH) is POSIX; the 3-arg capture form is a gawk
    # extension that errors on mawk (the default awk on Debian / Raspberry Pi OS).
    first_device="$(printf '%s\n' "${listing}" \
        | awk '/^card /{ if (match($0, /device [0-9]+/)) print substr($0, RSTART + 7, RLENGTH - 7); exit }')"

    # Prefer the card's ALSA *id* over its *index*.
    #
    # A card index is assigned in detection order and is not stable: it changes
    # when USB devices are re-enumerated, which a reboot is free to do. Measured
    # on a Raspberry Pi 4 during the acceptance run — the same microphone was
    # `card 1: PRO` before a cold reboot and `card 3: PRO` after it. The index
    # moved; the id did not. A station configured with the index came back from
    # that reboot serving a healthy dashboard and recording nothing, retrying a
    # device that no longer existed, forever.
    #
    # `plughw:CARD=<id>,DEV=<n>` addresses the card by that id. alsa-lib's own
    # alsa.conf declares `pcm.plughw { @args [ CARD DEV SUBDEV ] }` with
    # `@args.CARD { type string }`, forwarded to a `type hw` slave as
    # `card $CARD`, so a name is a first-class argument rather than a trick.
    #
    # This is the same identity `usb-audio-mapper` pins via a udev rule
    # (`ATTR{id}="<friendly_name>"`), so a station set up with that tool gets a
    # name the operator chose, and one that survives identical devices being
    # swapped between ports. See docs/book/admin/audio.md.
    #
    # Fall back to the index when the id cannot be trusted to identify one card:
    # two cards sharing an id would make `CARD=` ambiguous, and a non-portable
    # id would need quoting we cannot guarantee downstream.
    local id_count=0
    if [ -n "${first_id}" ]; then
        id_count="$(printf '%s\n' "${listing}" \
            | awk -v id="${first_id}" '$1 == "card" && $3 == id { n++ } END { print n+0 }')"
    fi
    if [ -n "${first_id}" ] \
        && printf '%s' "${first_id}" | grep -qE '^[A-Za-z0-9_-]+$' \
        && [ "${id_count}" = "1" ]; then
        echo "plughw:CARD=${first_id},DEV=${first_device:-0}"
    else
        echo "plughw:${first_card},${first_device:-0}"
    fi
}

# True (0) if v is a decimal number within [lo, hi]. Used to sanity-check
# latitude/longitude so a typo doesn't get written into the config.
valid_coord() {
    awk -v v="$1" -v lo="$2" -v hi="$3" \
        'BEGIN { if (v ~ /^[-+]?([0-9]+\.?[0-9]*|\.[0-9]+)$/ && v+0 >= lo && v+0 <= hi) exit 0; exit 1 }'
}

# Generate a strong, shell/URL-friendly random password.
gen_password() {
    if command -v openssl &>/dev/null; then
        openssl rand -base64 18 2>/dev/null | tr -dc 'A-Za-z0-9' | cut -c1-22
    else
        LC_ALL=C tr -dc 'A-Za-z0-9' </dev/urandom | head -c 22
    fi
}

# Guarantee the /admin panel is password-protected on a fresh LAN install. The
# dashboard binds to the LAN by default and viewing is open, but admin actions
# (settings, software update) must require a password — so if the operator
# didn't set one during onboarding, generate a strong one. No-ops when:
#   - the config already exists (never touch an operator's existing credentials)
#   - a password was already chosen during onboarding
#   - the dashboard is bound to localhost only (admin exposure is local anyway)
ensure_admin_password() {
    [ -f "${CONFIG_FILE}" ] && return 0
    [ -n "${CADDY_PWD_VALUE}" ] && return 0
    case "${LISTEN_ADDR}" in 127.0.0.1:* | localhost:*) return 0 ;; esac

    CADDY_USER_VALUE="admin"
    CADDY_PWD_VALUE="$(gen_password)"
    GENERATED_ADMIN_PASSWORD="${CADDY_PWD_VALUE}"
    info "Generated an admin password for the dashboard (shown at the end)."
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
        while :; do
            audio_in="$(ask "  Audio source — ALSA device (e.g. plughw:1,0) or rtsp:// URL (Enter to skip)" "")"
            case "${audio_in}" in
                '')
                    break ;;
                rtsp://?* | rtsps://?*)
                    RTSP_URL_VALUE="${audio_in}"; break ;;
                *://*)
                    # URL-like input whose scheme isn't rtsp(s):// is almost
                    # certainly a typo (e.g. http://…). Reject and re-prompt
                    # rather than silently storing it as an ALSA device string
                    # (which the capture path could never open). ALSA devices
                    # (plughw:1,0, hw:0, default) contain no '://', so this only
                    # catches mistyped stream URLs.
                    warn "  A stream URL must start with rtsp:// or rtsps:// — got '${audio_in}'. Try again." ;;
                *)
                    ALSA_CARD_VALUE="${audio_in}"; break ;;
            esac
        done
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

    # ---- Web dashboard ----
    printf '\n  Web dashboard\n' >/dev/tty
    printf '  The dashboard is reachable from other devices on your network. Viewing is\n' >/dev/tty
    printf '  open; the admin panel (settings, software update) is protected by a password.\n' >/dev/tty
    local pw1 pw2
    pw1="$(ask_secret "  Set an admin password now (Enter to auto-generate one)")"
    if [ -n "${pw1}" ]; then
        pw2="$(ask_secret "  Confirm password")"
        if [ "${pw1}" = "${pw2}" ]; then
            CADDY_USER_VALUE="admin"
            CADDY_PWD_VALUE="${pw1}"
            success "Admin password set (username: admin)."
        else
            warn "Passwords did not match — a strong one will be generated instead."
        fi
    fi
    # The dashboard intentionally binds all interfaces (0.0.0.0:8502) so it is
    # reachable from a phone or laptop out of the box. Restricting it to this
    # host is an advanced, easy-to-misfire choice — it strands a non-technical
    # operator who then "can't open the page" — so it is deliberately NOT a
    # setup-wizard question. Advanced users opt in explicitly by setting
    # BIRDNET_LISTEN=127.0.0.1:8502 in the environment or the config file, which
    # resolve_listen_addr honours (and the installer preserves across re-runs).
}
