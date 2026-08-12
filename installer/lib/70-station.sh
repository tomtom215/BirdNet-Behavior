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

# Read whatever the operator typed at the coordinate prompt and print either
# "lat lon" or a bare "lat", or nothing at all if it is not coordinates.
#
# The prompt used to accept exactly one shape — a lone dotted decimal — which
# was at odds with the tip printed directly above it: right-clicking on
# OpenStreetMap hands you a *pair*, "49.4521, 8.6724", and pasting that was
# rejected. It was also at odds with the rest of the product, since the web
# settings form accepts a decimal comma. Both now parse:
#
#   49.4521             a lone latitude; the longitude is asked for next
#   49.4521, 8.6724     the pair OpenStreetMap gives you
#   49,4521             decimal comma
#   49,4521 8,6724      decimal commas, space separated
#   49,4521,8,6724      decimal commas, comma separated
#
# The one real ambiguity is a single comma: is "49,45" the decimal 49.45 or the
# pair (49, 45)? It resolves on range wherever it can — in "49,4521" the tail
# 4521 is not a valid longitude, so the comma has to be a decimal point. Where
# both readings are valid the pair wins, matching the OpenStreetMap flow the
# tip sends people to; the caller echoes the result back so a wrong guess is
# visible and correctable rather than silently written to the config.
parse_coords() {
    awk -v s="$1" '
        function strip(x) { sub(/,$/, "", x); return x }
        function norm(x)  { gsub(/,/, ".", x); return x }
        function ok(x, lo, hi) {
            return x ~ /^[-+]?([0-9]+\.?[0-9]*|\.[0-9]+)$/ && x + 0 >= lo && x + 0 <= hi
        }
        BEGIN {
            gsub(/^[ \t]+|[ \t]+$/, "", s)
            n = split(s, tok, /[ \t]+/)

            if (n == 2) {
                a = norm(strip(tok[1])); b = norm(tok[2])
                if (ok(a, -90, 90) && ok(b, -180, 180)) print a " " b
                exit
            }
            if (n != 1) exit

            t = tok[1]
            if (index(t, ",") == 0) {            # a plain dotted decimal
                if (ok(t, -90, 90)) print t
                exit
            }
            m = split(t, p, ",")
            if (m == 2) {
                if (ok(p[1], -90, 90) && ok(p[2], -180, 180)) { print p[1] " " p[2]; exit }
                c = p[1] "." p[2]                # so the comma was a decimal point
                if (ok(c, -90, 90)) print c
                exit
            }
            if (m == 4) {                        # "49,4521,8,6724"
                a = p[1] "." p[2]; b = p[3] "." p[4]
                if (ok(a, -90, 90) && ok(b, -180, 180)) print a " " b
            }
        }'
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
    #
    # This loops, like the audio-source prompt above it. It used to warn once
    # and fall through, which meant unreadable input was discarded into a
    # single [WARN] line that scrolled away behind the 541 MB model download —
    # the operator answered the question, the station recorded no location, and
    # nothing said so again until the species filter silently stayed off.
    printf '\n  Station location (solar schedule, species filter, BirdWeather)\n' >/dev/tty
    printf '  Tip: right-click your spot on https://openstreetmap.org and read off the coordinates.\n' >/dev/tty
    printf '  Paste both at once (49.4521, 8.6724) or enter them one at a time.\n' >/dev/tty
    printf '  A decimal comma (49,4521) is fine.\n' >/dev/tty
    local coord_in lon_in coords
    while :; do
        coord_in="$(ask "  Latitude — or both coordinates (Enter to skip)" "")"
        if [ -z "${coord_in}" ]; then
            warn "No location set — species filtering stays off until you set one."
            warn "  Add LATITUDE/LONGITUDE to ${CONFIG_FILE}, or use the dashboard's setup wizard."
            break
        fi
        coords="$(parse_coords "${coord_in}")"
        if [ -z "${coords}" ]; then
            warn "  Could not read '${coord_in}' as coordinates — try again, or press Enter to skip."
            continue
        fi
        # A lone latitude: collect its other half, then re-parse the two
        # together so the pair goes through exactly one validation path.
        case "${coords}" in
            *' '*) : ;;
            *)
                lon_in="$(ask "  Longitude (e.g. 8.6724)" "")"
                coords="$(parse_coords "${coords} ${lon_in}")"
                ;;
        esac
        case "${coords}" in
            *' '*)
                LATITUDE_VALUE="${coords%% *}"
                LONGITUDE_VALUE="${coords##* }"
                success "Location: ${LATITUDE_VALUE}, ${LONGITUDE_VALUE}"
                break
                ;;
            *)
                warn "  A latitude on its own does not locate the station — try again, or press Enter to skip."
                ;;
        esac
    done

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
