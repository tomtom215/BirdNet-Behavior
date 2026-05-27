# ---------------------------------------------------------------------------
# Print post-install instructions
# ---------------------------------------------------------------------------

print_summary() {
    local ip web_host
    ip="$(hostname -I 2>/dev/null | awk '{print $1}' || echo 'localhost')"
    # Show the address the dashboard actually answers on.
    case "${LISTEN_ADDR}" in
        127.0.0.1:* | localhost:*) web_host="localhost" ;;
        *)                         web_host="${ip}" ;;
    esac

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
    echo -e "  ${BOLD}Web UI:${RESET}  http://${web_host}:8502"
    echo
    if systemctl is-active --quiet birdnet-behavior.service 2>/dev/null; then
        echo -e "${GREEN}Your dashboard is live${RESET} — open a web browser to:  ${BOLD}http://${web_host}:8502${RESET}"
        [ "${web_host}" != "localhost" ] && echo "  (reachable from any device on your network)"
    else
        echo -e "${BOLD}Next steps:${RESET}"
        echo "  1. Set an audio source (edit as root):  sudo nano ${CONFIG_FILE}"
        echo "       ALSA_CARD=plughw:1,0      (ALSA microphone)"
        echo "       RTSP_URL=rtsp://…         (RTSP camera)"
        echo
        echo "  2. (Optional) Set LATITUDE and LONGITUDE for species filtering."
        echo
        echo "  3. sudo systemctl start birdnet-behavior"
        echo "  4. Open a web browser to  http://${web_host}:8502"
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
