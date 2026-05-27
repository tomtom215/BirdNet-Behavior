# ---------------------------------------------------------------------------
# Print post-install instructions
# ---------------------------------------------------------------------------

print_summary() {
    local ip web_host mdns_host browse_host
    ip="$(hostname -I 2>/dev/null | awk '{print $1}' || echo 'localhost')"
    # Show the address the dashboard actually answers on.
    case "${LISTEN_ADDR}" in
        127.0.0.1:* | localhost:*) web_host="localhost" ;;
        *)                         web_host="${ip}" ;;
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
    browse_host="${mdns_host:-${web_host}}"

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
    echo -e "  ${BOLD}Web UI:${RESET}  http://${browse_host}:8502"
    [ -n "${mdns_host}" ] && echo -e "           http://${web_host}:8502  (same dashboard, by IP — if the .local name doesn't resolve)"
    echo
    if systemctl is-active --quiet birdnet-behavior.service 2>/dev/null; then
        echo -e "${GREEN}Your dashboard is live${RESET} — open a web browser to:  ${BOLD}http://${browse_host}:8502${RESET}"
        if [ -n "${mdns_host}" ]; then
            echo "  (or http://${web_host}:8502 by IP — reachable from any device on your network)"
        elif [ "${web_host}" != "localhost" ]; then
            echo "  (reachable from any device on your network)"
        fi
    else
        echo -e "${BOLD}Next steps:${RESET}"
        echo "  1. Set an audio source (edit as root):  sudo nano ${CONFIG_FILE}"
        echo "       ALSA_CARD=plughw:1,0      (ALSA microphone)"
        echo "       RTSP_URL=rtsp://…         (RTSP camera)"
        echo
        echo "  2. (Optional) Set LATITUDE and LONGITUDE for species filtering."
        echo
        echo "  3. sudo systemctl start birdnet-behavior"
        echo "  4. Open a web browser to  http://${browse_host}:8502"
    fi

    # Admin login. Viewing the dashboard is open; the admin panel (settings +
    # software update) needs these credentials.
    if [ -n "${GENERATED_ADMIN_PASSWORD}" ]; then
        echo
        echo -e "  ${BOLD}Admin panel login${RESET} (settings + software update — viewing is open):"
        echo -e "      username:  ${BOLD}birdnet${RESET}"
        echo -e "      password:  ${BOLD}${GENERATED_ADMIN_PASSWORD}${RESET}"
        echo    "      (auto-generated, saved as CADDY_PWD in ${CONFIG_FILE} — change it any time)"
    elif [ -n "${CADDY_PWD_VALUE}" ]; then
        echo
        echo "  Admin panel (settings + software update): sign in as 'birdnet' with the password you set."
    fi
    echo
    echo "  Logs:  sudo journalctl -u birdnet-behavior -f"
    echo
}
