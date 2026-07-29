# ---------------------------------------------------------------------------
# Privilege, architecture, and glibc preflight
# ---------------------------------------------------------------------------

# Whether systemd is the running init, so `systemctl` calls will actually work.
#
# `systemctl` can be present on a system where systemd is NOT PID 1 — minimal
# containers, chroots, WSL1, some CI runners — and there every systemctl call
# fails. `/run/systemd/system` is systemd's own "I am running" marker, so this
# is the canonical guard. When it returns false the installer writes the unit
# but skips enable/start, degrading cleanly instead of aborting (see
# install_service / maybe_start_service).
has_systemd() {
    command -v systemctl >/dev/null 2>&1 && [ -d /run/systemd/system ]
}

require_root() {
    if [ "$(id -u)" -ne 0 ]; then
        error "This installer needs root — it installs a systemd service."
        cat >&2 <<EOF

Re-run it by piping into sudo:

    curl -fsSL https://raw.githubusercontent.com/${REPO}/main/install.sh | sudo bash

To pin a version, pass it as an argument after a literal --:

    curl -fsSL https://raw.githubusercontent.com/${REPO}/main/install.sh | sudo bash -s -- --version ${VERSION:-X.Y.Z}

Already saved the script?   sudo bash install.sh [--version ${VERSION:-X.Y.Z}]

Avoid  sudo bash <(curl ...)  — process substitution gives bash a file
descriptor owned by your user, and sudo closes it on the way to root, so the
script vanishes ("/dev/fd/63: No such file or directory"). The pipe above works.
EOF
        exit 1
    fi
    # Determine who to run the service as.  When invoked via `sudo`, $SUDO_USER
    # is the original (non-root) user.  Refuse to run as root directly so the
    # service doesn't end up owned by root.
    if [ -z "${SUDO_USER:-}" ] || [ "${SUDO_USER}" = "root" ]; then
        fatal "Run the installer via sudo from a normal user account, not as root directly, so the service isn't owned by root.  E.g.:  curl -fsSL https://raw.githubusercontent.com/${REPO}/main/install.sh | sudo bash"
    fi
    SERVICE_USER="${SUDO_USER}"

    # Under sudo, $HOME is usually /root, not the service user's home — so the
    # data dir computed at the top of this script can land in /root, which the
    # non-root service user cannot reach (and ProtectHome=read-only would block
    # it anyway). Re-derive every home-based path from the service user's actual
    # home so the daemon can read its database, recordings, and model.
    local svc_home
    svc_home="$(getent passwd "${SERVICE_USER}" | cut -d: -f6)"
    if [ -n "${svc_home}" ]; then
        DATA_DIR="${svc_home}/BirdNet-Behavior"
        RECS_DIR="${DATA_DIR}/recordings"
        IMAGE_CACHE_DIR="${DATA_DIR}/image_cache"
        MODEL_DIR="${DATA_DIR}/models"
        DB_PATH="${DATA_DIR}/birds.db"
    fi
}

detect_arch() {
    local machine
    machine="$(uname -m)"
    case "${machine}" in
        aarch64 | arm64) echo "aarch64-unknown-linux-gnu" ;;
        x86_64)          echo "x86_64-unknown-linux-gnu" ;;
        armv6l | armv7l)
            # The `ort` crate does not ship prebuilt ONNX Runtime binaries
            # for armv7 / armv6, so BirdNet-Behavior does not publish a
            # 32-bit ARM release binary.  Every modern Raspberry Pi (Pi 3,
            # Pi 4, Pi 5, Pi 400, Pi Zero 2 W) is 64-bit-capable — if you
            # are seeing this message you are almost certainly running the
            # 32-bit Pi OS on 64-bit-capable hardware.  The fix is to
            # reflash with the 64-bit image, which is the Pi Foundation's
            # current recommendation anyway.
            cat >&2 <<'EOT'

  Unsupported architecture: 32-bit ARM (armv6l / armv7l).

  BirdNet-Behavior does not publish 32-bit ARM release binaries because
  the ONNX Runtime crate (`ort`) ships no prebuilt libraries for that
  target.

  If you are on a Raspberry Pi 3, 4, 5, 400, or Zero 2 W, your hardware
  is 64-bit-capable and you are just running the 32-bit Pi OS.  Reflash
  with the 64-bit image (this is the current Raspberry Pi Foundation
  recommendation) and re-run this installer:

      https://downloads.raspberrypi.com/raspios_arm64/images/

  After reflashing, `uname -m` should print `aarch64`, and this script
  will download the aarch64 release binary automatically.

  Only the original 2015 Pi 2 v1.1 (Cortex-A7) and the Pi 1 / Pi Zero /
  Pi Zero W (ARM11) lack a 64-bit mode entirely.  Those boards would
  need a from-source build, which is not currently supported upstream.

EOT
            fatal "Unsupported architecture: ${machine}."
            ;;
        *)
            fatal "Unsupported architecture: ${machine}. Supported: aarch64, x86_64."
            ;;
    esac
}

# The prebuilt release binaries are built on Ubuntu 24.04 (glibc 2.39) because
# pyke's prebuilt ONNX Runtime requires glibc >= 2.38. On an older system
# (notably Raspberry Pi OS Bookworm / Debian 12, glibc 2.36) the binary loads
# but dies with "version `GLIBC_2.39' not found". Catch that here, before we
# download 540 MB of model, and point the user at a path that actually works.
detect_glibc_version() {
    # Prefer getconf (prints "glibc 2.36"); fall back to `ldd --version`.
    local v
    v="$(getconf GNU_LIBC_VERSION 2>/dev/null | awk '{print $2}')"
    if [ -z "${v}" ]; then
        v="$(ldd --version 2>/dev/null | head -1 | grep -oE '[0-9]+\.[0-9]+' | tail -1)"
    fi
    echo "${v}"
}

check_glibc() {
    if [ "${BIRDNET_SKIP_GLIBC_CHECK:-0}" = "1" ]; then
        loud_warn "BIRDNET_SKIP_GLIBC_CHECK=1 — glibc compatibility check BYPASSED." \
                  "If this system's glibc is older than ${REQUIRED_GLIBC}, the daemon will" \
                  "crash at startup with a 'GLIBC_${REQUIRED_GLIBC} not found' error." \
                  "Unset BIRDNET_SKIP_GLIBC_CHECK to re-enable the check."
        return 0
    fi

    local current
    current="$(detect_glibc_version)"
    if [ -z "${current}" ]; then
        warn "Could not determine the system glibc version."
        warn "The prebuilt binary needs glibc >= ${REQUIRED_GLIBC}; continuing anyway."
        return 0
    fi

    # current >= required  iff  the lower of the two (sort -V) is the requirement.
    if [ "$(printf '%s\n%s\n' "${REQUIRED_GLIBC}" "${current}" | sort -V | head -1)" = "${REQUIRED_GLIBC}" ]; then
        success "glibc ${current} (>= ${REQUIRED_GLIBC}) — OK"
        return 0
    fi

    error "System glibc is ${current}, but the prebuilt binary requires >= ${REQUIRED_GLIBC}."
    cat >&2 <<EOT

  Your OS is too old for the prebuilt native binary. Raspberry Pi OS
  Bookworm / Debian 12 ship glibc 2.36 and are NOT supported by the
  binary. Two paths that DO work:

    1. Run the Docker image — it bundles its own runtime, so the host
       glibc does not matter (works fine on Bookworm):

           bash <(curl -fsSL https://raw.githubusercontent.com/${REPO}/main/quickstart.sh)

    2. Upgrade the OS to Raspberry Pi OS Trixie / Debian 13 /
       Ubuntu 24.04 (or newer), then re-run this installer.

  Building from source against your system's older toolchain also works;
  see the documentation. To bypass this check anyway (you know what you
  are doing), re-run with BIRDNET_SKIP_GLIBC_CHECK=1.

EOT
    fatal "Unsupported glibc ${current} (need >= ${REQUIRED_GLIBC})."
}
