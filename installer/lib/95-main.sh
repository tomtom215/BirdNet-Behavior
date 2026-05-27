# ---------------------------------------------------------------------------
# Main entry point
# ---------------------------------------------------------------------------

main() {
    parse_args "$@"

    # Prompt only with a real terminal to read from. Under `curl ... | sudo bash`
    # stdin is the pipe, but stdout (fd 1) and /dev/tty are still the user's
    # terminal — so gate on those, not on stdin.
    if [ "${BIRDNET_NONINTERACTIVE:-0}" != "1" ] && [ -t 1 ] && [ -r /dev/tty ]; then
        INTERACTIVE=1
    fi

    echo -e "${BOLD}BirdNet-Behavior Installer${RESET}"
    echo "  Repository: https://github.com/${REPO}"
    echo

    # macOS is not systemd — dispatch to the per-user launchd path before any
    # root check, download, or filesystem change, so a Mac user never gets a
    # half-finished Linux install.
    if [ "$(uname -s)" = "Darwin" ]; then
        if [ "${SUBCOMMAND}" = "uninstall" ]; then
            warn "On macOS, uninstall with:  ./uninstall.sh   (it handles the launchd LaunchAgent)."
            exit 0
        fi
        macos_install
        exit 0
    fi

    require_root
    detect_existing_install

    # No explicit command: a fresh box installs; an existing one offers the
    # menu interactively, or silently updates when non-interactive (preserving
    # the historical `curl | sudo bash` auto-upgrade behaviour for automation).
    if [ -z "${SUBCOMMAND}" ]; then
        if [ "${EXISTING_INSTALL}" = 1 ]; then
            if [ "${INTERACTIVE}" = 1 ]; then
                choose_existing_action
            else
                info "Existing install detected — updating (non-interactive)."
                SUBCOMMAND="update"
            fi
        else
            SUBCOMMAND="install"
        fi
    fi

    MODE="${SUBCOMMAND}"
    case "${SUBCOMMAND}" in
        install | update | reinstall) do_install ;;
        repair)                       do_repair ;;
        uninstall)                    do_uninstall ;;
        *)                            fatal "Unknown command: ${SUBCOMMAND}" ;;
    esac
}

main "$@"
