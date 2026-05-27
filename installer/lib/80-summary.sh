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
        if [ "${web_host}" = "localhost" ]; then
            echo -e "${GREEN}Your dashboard is live.${RESET}  On THIS device, open a web browser to:"
            echo -e "      ${BOLD}http://localhost:8502${RESET}"
            echo "  To open it from your phone or laptop, re-run the installer and answer yes to"
            echo "  \"reachable from other devices\", or set BIRDNET_LISTEN=0.0.0.0:8502."
        else
            echo -e "${GREEN}Your dashboard is live.${RESET}  From a phone or computer on the same"
            echo -e "  network, open a web browser to:  ${BOLD}http://${web_host}:8502${RESET}"
            [ -n "${CADDY_PWD_VALUE}" ] && echo "  Sign in with username 'birdnet' and the password you just set."
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
        echo "  4. Open a web browser to  http://${web_host}:8502"
    fi
    echo
    echo "  Logs:  sudo journalctl -u birdnet-behavior -f"
    echo
}
