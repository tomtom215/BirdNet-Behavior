# ---------------------------------------------------------------------------
# Argument parsing and usage
# ---------------------------------------------------------------------------

usage() {
    cat <<EOF
BirdNet-Behavior installer

Linux / Raspberry Pi (installs a systemd service, so it needs root):
  curl -fsSL https://raw.githubusercontent.com/${REPO}/main/install.sh | sudo bash
  curl -fsSL https://raw.githubusercontent.com/${REPO}/main/install.sh | sudo bash -s -- --version X.Y.Z

From a saved copy of this script:
  sudo bash install.sh [--version X.Y.Z]

Commands (auto-offered as a menu when an existing install is detected):
  install        Fresh install (the default when nothing is installed).
  update         Install the latest (or --version) binary; keep all settings.
  repair         Recreate directories, fix permissions, rewrite the systemd
                 unit, and restart. Fixes a service that won't start without
                 re-downloading anything.
  reinstall      Re-download the binary and rewrite the unit/config
                 (your data and the model are preserved).
  uninstall      Remove the software (binary, service, tmpfs unit); keep data.

Options:
  -v, --version X.Y.Z   Install a specific release (default: latest stable).
                        The VERSION environment variable is still honoured too.
      --noninteractive  Don't prompt; auto-detect audio and leave location
                        unset (also implied by BIRDNET_NONINTERACTIVE=1 or no TTY).
                        With an existing install this implies 'update'.
  -h, --help            Show this help and exit.

Avoid  sudo bash <(curl ...)  — sudo closes the process-substitution file
descriptor on the way to root, so the script never loads. Use the pipe above.
EOF
}

parse_args() {
    while [ "$#" -gt 0 ]; do
        case "$1" in
            -v | --version)
                [ "$#" -ge 2 ] || fatal "--version needs a value, e.g. --version 0.5.1"
                VERSION="$2"
                shift 2
                ;;
            --version=*)
                VERSION="${1#*=}"
                shift
                ;;
            --noninteractive | --non-interactive)
                BIRDNET_NONINTERACTIVE=1
                shift
                ;;
            -h | --help)
                usage
                exit 0
                ;;
            install | update | repair | reinstall | uninstall)
                [ -z "${SUBCOMMAND}" ] || fatal "Specify only one command (got '${SUBCOMMAND}' and '$1')."
                SUBCOMMAND="$1"
                shift
                ;;
            --)
                shift
                ;;
            -*)
                fatal "Unknown option: $1  (run with --help for usage)."
                ;;
            *)
                fatal "Unexpected argument: $1  (run with --help for usage)."
                ;;
        esac
    done
}
