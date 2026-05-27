# install.sh — BirdNet-Behavior installer for Raspberry Pi and x86_64 Linux
#
# Usage (Linux / Raspberry Pi — installs a systemd service, so it needs root):
#   curl -fsSL https://raw.githubusercontent.com/tomtom215/BirdNet-Behavior/main/install.sh | sudo bash
#   # pin a specific version (the `-s --` passes the args through to the script):
#   curl -fsSL https://raw.githubusercontent.com/tomtom215/BirdNet-Behavior/main/install.sh | sudo bash -s -- --version 0.5.1
#   # from a saved copy:
#   sudo bash install.sh [--version 0.5.1]
#
# When an existing install is detected the script offers update / repair /
# reinstall / uninstall. You can also pick one explicitly:
#   sudo bash install.sh update       # swap in the latest binary, keep settings
#   sudo bash install.sh repair       # fix dirs/permissions + rewrite the unit
#   sudo bash install.sh reinstall    # re-download and rewrite everything
#   sudo bash install.sh uninstall    # remove the software, keep data
#
# Do NOT use `sudo bash <(curl ...)`: process substitution hands bash a file
# descriptor owned by your user, and sudo closes it crossing to root, so the
# script disappears ("/dev/fd/63: No such file or directory"). Use the pipe.
#
# macOS (Apple Silicon) sets up a per-user launchd agent instead — run without sudo.
#
# What this script does on a fresh install:
#   1. Pre-flight checks (architecture, glibc, required tools, free disk)
#   2. Downloads + checksum-verifies the pre-built binary from GitHub Releases
#   3. Creates configuration, data, and recording directories
#   4. Installs a hardened systemd service unit (birdnet-behavior.service)
#   5. Optionally prompts for ALSA device / RTSP URL / location
#   6. Post-install validation (binary, unit, directories, doctor, port)
#
# Every step is idempotent — re-running the script is always safe.
#
# Requirements: curl or wget, tar, sha256sum, systemd
