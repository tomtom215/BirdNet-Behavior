# ---------------------------------------------------------------------------
# Print post-install instructions
# ---------------------------------------------------------------------------

# Print a scannable QR of the dashboard URL when possible, so a phone can open
# the dashboard without anyone typing an IP. Best-effort: needs qrencode and a
# LAN-reachable host (a localhost-only bind has nothing for another device to
# scan). The IP URL is encoded on purpose — it always resolves on the LAN,
# whereas mDNS `.local` is not universal (some phones / networks never resolve it).
print_dashboard_qr() {
    local host="$1" port="$2"
    [ "${host}" = "localhost" ] && return 0
    command -v qrencode &>/dev/null || return 0
    echo
    echo -e "  ${BOLD}Scan to open on your phone${RESET} (same Wi-Fi network):"
    qrencode -t ANSIUTF8 -m 2 "http://${host}:${port}" 2>/dev/null | sed 's/^/    /' || true
}

# Admin login. Viewing the dashboard is open; the admin panel (settings,
# software update, system controls) requires signing in.
#
# This has to work on an UPGRADE, not only on a fresh install.
# GENERATED_ADMIN_PASSWORD and CADDY_PWD_VALUE are set only during onboarding,
# and onboarding is skipped whenever a config already exists
# (`70-station.sh` returns early on `[ -f "${CONFIG_FILE}" ]`). So on every
# update/repair/reinstall of an existing station both were empty and this block
# printed NOTHING — no username, no mention that a password exists at all.
#
# That is a lockout, not a cosmetic gap. This release closes an `/admin` that
# was served unauthenticated, so an operator who has never needed the password
# since install now needs one they were shown once, months ago, in scrollback
# that is long gone — and the only route back is a root grep of a file they
# have no reason to know exists.
#
# The password itself is deliberately NOT reprinted here. It is already in the
# operator's terminal history from install time, and an upgrade is not a reason
# to spray it into scrollback again. The exact command to reveal it is printed
# instead, so the recovery path is one copy-paste rather than a search.
print_admin_login() {
    if [ -n "${GENERATED_ADMIN_PASSWORD}" ]; then
        echo
        echo -e "  ${BOLD}Admin panel login${RESET} (settings + software update — viewing is open):"
        echo -e "      username:  ${BOLD}admin${RESET}"
        echo -e "      password:  ${BOLD}${GENERATED_ADMIN_PASSWORD}${RESET}"
        echo    "      (auto-generated, saved as CADDY_PWD in ${CONFIG_FILE} — change it any time)"
        echo    "      Save it now — it is not shown again."
        return 0
    fi
    if [ -n "${CADDY_PWD_VALUE}" ]; then
        echo
        echo -e "  ${BOLD}Admin panel login${RESET} (settings + software update — viewing is open):"
        echo -e "      username:  ${BOLD}admin${RESET}"
        echo    "      password:  the one you just set."
        return 0
    fi

    # Nothing was generated or entered this run — an upgrade or a repair. Read
    # the station's own config to say something useful rather than nothing.
    if grep -qE '^[[:space:]]*CADDY_PWD[[:space:]]*=[[:space:]]*[^[:space:]#]' \
        "${CONFIG_FILE}" 2>/dev/null; then
        echo
        echo -e "  ${BOLD}Admin panel login${RESET} (settings + software update — viewing stays open):"
        echo -e "      username:  ${BOLD}admin${RESET}"
        echo    "      password:  already set on this station. To see it:"
        echo -e "        ${BOLD}sudo grep '^CADDY_PWD' ${CONFIG_FILE}${RESET}"
        echo    "      Signing in is required for the admin panel — earlier versions"
        echo    "      served it without a password. Viewing the dashboard is unchanged."
        return 0
    fi

    # No password at all. On a LAN-reachable station that means the admin panel
    # is open to anyone who can load the dashboard, which the operator should
    # hear about loudly rather than discover.
    case "${LISTEN_ADDR}" in
        127.0.0.1:* | localhost:*)
            echo
            echo "  Admin panel: no password set (CADDY_PWD). The dashboard is bound to"
            echo "  this device only, so the panel is reachable just from here."
            ;;
        *)
            loud_warn \
                "The admin panel has NO PASSWORD and the dashboard is on your network." \
                "Anyone who can reach it can change settings and update the software." \
                "Fix: set CADDY_PWD in ${CONFIG_FILE}, then:" \
                "  sudo bash install.sh repair"
            ;;
    esac
    return 0
}

print_summary() {
    local ip web_host mdns_host
    ip="$(hostname -I 2>/dev/null | awk '{print $1}' || echo 'localhost')"
    # Show the address — and port — the dashboard actually answers on, so an
    # operator who set a custom BIRDNET_LISTEN sees the right URL, not the default.
    case "${LISTEN_ADDR}" in
        127.0.0.1:* | localhost:*) web_host="localhost" ;;
        *)                         web_host="${ip}" ;;
    esac
    local web_port="${LISTEN_ADDR##*:}"
    case "${web_port}" in
        '' | *[!0-9]*) web_port="8502" ;; # no/invalid port → canonical default
    esac

    # Best-effort mDNS name. Pi OS ships avahi, so http://<hostname>.local is a
    # more durable bookmark than a DHCP-assigned IP (which can change on the next
    # lease). Only meaningful when the dashboard is exposed beyond localhost;
    # clients without an mDNS resolver fall back to the IP shown beside it.
    mdns_host=""
    if [ "${web_host}" != "localhost" ]; then
        local short
        short="$(hostname -s 2>/dev/null || true)"
        [ -n "${short}" ] && mdns_host="${short}.local"
    fi

    local headline="Installation complete!"
    case "${MODE}" in
        update)    headline="Update complete!" ;;
        reinstall) headline="Reinstall complete!" ;;
    esac

    echo
    echo -e "${BOLD}${GREEN}${headline}${RESET}"
    echo
    echo -e "  ${BOLD}Binary:${RESET}  ${INSTALL_DIR}/${BINARY_NAME}"
    echo -e "  ${BOLD}Config:${RESET}  ${CONFIG_FILE}"
    echo -e "  ${BOLD}Data:${RESET}    ${DATA_DIR}"
    # Lead with the IP address: it works for every device on the LAN. The mDNS
    # `.local` name is friendlier but NOT universal — some phones and networks
    # don't resolve it — so it is shown as a clearly-secondary convenience, never
    # the only address the operator is handed.
    echo -e "  ${BOLD}Web UI:${RESET}  http://${web_host}:${web_port}   (works from any device on your network)"
    [ -n "${mdns_host}" ] && echo -e "           http://${mdns_host}:${web_port}   (friendlier name — works on most devices, but not all)"
    print_dashboard_qr "${web_host}" "${web_port}"
    echo
    if systemctl is-active --quiet birdnet-behavior.service 2>/dev/null; then
        echo -e "${GREEN}Your dashboard is live${RESET} — open a web browser to:  ${BOLD}http://${web_host}:${web_port}${RESET}"
        [ "${web_host}" != "localhost" ] && echo "  (reachable from any device on your network)"
        # Live, but a station with no audio source won't detect anything yet.
        # Point the operator at the in-dashboard setup wizard (and the config
        # fallback) so it's clear why no birds are showing up.
        if ! config_has_audio_source; then
            echo
            echo "  No audio source yet, so no birds will be detected. Open the dashboard"
            echo "  to pick a microphone in the setup wizard — or set ALSA_CARD / RTSP_URL"
            echo "  in ${CONFIG_FILE} and:  sudo systemctl restart birdnet-behavior"
        fi
        # Live and listening, but with no coordinates the species filter cannot
        # run — so the station reports birds from the wrong continent and looks
        # like a bad model. Silence here was the whole problem: unlike a missing
        # microphone, this failure produces detections, just wrong ones.
        if ! config_has_location; then
            echo
            echo "  No station location set. Without it every species in the model stays a"
            echo "  candidate, so expect detections that don't belong in your area. Set it"
            echo "  in the dashboard's setup wizard (it can auto-detect), or add LATITUDE"
            echo "  and LONGITUDE to ${CONFIG_FILE} and:  sudo systemctl restart birdnet-behavior"
        fi
    else
        echo -e "${BOLD}Next steps:${RESET}"
        echo "  1. Set an audio source (edit as root):  sudo nano ${CONFIG_FILE}"
        echo "       ALSA_CARD=plughw:1,0      (ALSA microphone)"
        echo "       RTSP_URL=rtsp://…         (RTSP camera)"
        echo
        echo "  2. Set LATITUDE and LONGITUDE. Without them the species filter cannot"
        echo "     run and you will get detections from the wrong part of the world."
        echo
        echo "  3. sudo systemctl start birdnet-behavior"
        echo "  4. Open a web browser to  http://${web_host}:${web_port}"
    fi

    print_admin_login
    echo
    echo "  Logs:  sudo journalctl -u birdnet-behavior -f"
    echo
}
