# ---------------------------------------------------------------------------
# Logging and interactive prompt helpers
# ---------------------------------------------------------------------------

# All logging goes to stderr so that stdout is reserved for a function's return
# value. resolve_version/detect_arch return via `echo`, and callers capture them
# with $(...) — a log line on stdout would be swallowed into the value (e.g. a
# version of "[INFO] Querying…\n0.5.1", which then corrupts the download URL).
info()    { echo -e "${BLUE}[INFO]${RESET}  $*" >&2; }
success() { echo -e "${GREEN}[OK]${RESET}    $*" >&2; }
warn()    { echo -e "${YELLOW}[WARN]${RESET}  $*" >&2; }
error()   { echo -e "${RED}[ERROR]${RESET} $*" >&2; }
fatal()   { error "$*"; exit 1; }

# Interactive prompt helpers. They read from /dev/tty (not stdin) so they work
# under the recommended `curl ... | sudo bash`, where stdin is the script text;
# output goes to /dev/tty for the same reason. Gated by INTERACTIVE (set in main).
ask() {
    local prompt="$1" default="${2:-}" reply
    if [ -n "${default}" ]; then
        printf '%s [%s]: ' "${prompt}" "${default}" >/dev/tty
    else
        printf '%s: ' "${prompt}" >/dev/tty
    fi
    read -r reply </dev/tty || reply=""
    printf '%s' "${reply:-${default}}"
}

yesno() {
    local prompt="$1" default="${2:-y}" reply hint="[Y/n]"
    [ "${default}" = "n" ] && hint="[y/N]"
    printf '%s %s ' "${prompt}" "${hint}" >/dev/tty
    read -r reply </dev/tty || reply=""
    case "${reply:-${default}}" in [yY]*) return 0 ;; *) return 1 ;; esac
}

# Like ask, but does not echo the input — for passwords. Reads from /dev/tty.
ask_secret() {
    local prompt="$1" reply
    printf '%s: ' "${prompt}" >/dev/tty
    read -rs reply </dev/tty || reply=""
    printf '\n' >/dev/tty
    printf '%s' "${reply}"
}
