# ---------------------------------------------------------------------------
# Argument parsing and usage
# ---------------------------------------------------------------------------

usage() {
    cat <<EOF
BirdNet-Behavior installer

Linux / Raspberry Pi (installs a systemd service, so it needs root):
  curl --proto '=https' --tlsv1.2 -fsSL https://github.com/${REPO}/releases/latest/download/install.sh | sudo bash
  curl --proto '=https' --tlsv1.2 -fsSL https://github.com/${REPO}/releases/latest/download/install.sh | sudo bash -s -- --version X.Y.Z
  curl --proto '=https' --tlsv1.2 -fsSL https://github.com/${REPO}/releases/latest/download/install.sh | sudo bash -s -- --classifier birdnet-v3

From a saved copy of this script:
  sudo bash install.sh [--version X.Y.Z] [--classifier birdnet-v24|birdnet-v3]

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
      --classifier ID   Which classifier a NEW install runs (also: BIRDNET_CLASSIFIER):
                          birdnet-v24  BirdNET V2.4 + its range filter v2 (default).
                                       The network BirdNET-Pi and birdnet-go run,
                                       so detections agree with theirs. ~85 MB.
                          birdnet-v3   BirdNET+ V3.0 preview + the BirdNET geomodel.
                                       A different network; detections differ from
                                       BirdNET-Pi's by design. ~555 MB.
                        An existing station keeps the classifier its config names:
                        install, update, repair and reinstall never change it.
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
            --classifier)
                [ "$#" -ge 2 ] || fatal "--classifier needs a value: ${CLASSIFIER_V24} or ${CLASSIFIER_V3}."
                CLASSIFIER_REQUEST="$2"
                shift 2
                ;;
            --classifier=*)
                CLASSIFIER_REQUEST="${1#*=}"
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
    # Refuse a typo now, before any download, whichever way it arrived (flag or
    # BIRDNET_CLASSIFIER). resolve_classifier checks again for the paths that
    # do not come through here.
    if [ -n "${CLASSIFIER_REQUEST}" ] && ! valid_classifier_id "${CLASSIFIER_REQUEST}"; then
        fatal "Unknown classifier '${CLASSIFIER_REQUEST}'. Choose ${CLASSIFIER_V24} (the default) or ${CLASSIFIER_V3}."
    fi
}
