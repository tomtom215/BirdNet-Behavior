# install.sh — BirdNet-Behavior installer for Raspberry Pi and x86_64 Linux
#
# Usage (Linux / Raspberry Pi — installs a systemd service, so it needs root):
#   curl --proto '=https' --tlsv1.2 -fsSL https://github.com/tomtom215/BirdNet-Behavior/releases/latest/download/install.sh | sudo bash
#   # pin a specific version (the `-s --` passes the args through to the script):
#   curl --proto '=https' --tlsv1.2 -fsSL https://github.com/tomtom215/BirdNet-Behavior/releases/latest/download/install.sh | sudo bash -s -- --version 0.5.1
#   # from a saved copy:
#   sudo bash install.sh [--version 0.5.1]
#   # choose the classifier of a NEW install (default: BirdNET V2.4):
#   sudo bash install.sh --classifier birdnet-v3
#
# When an existing install is detected the script offers update / repair /
# reinstall / uninstall. You can also pick one explicitly:
#   sudo bash install.sh update       # swap in the latest binary, keep settings
#   sudo bash install.sh repair       # fix dirs/permissions + rewrite the unit
#   sudo bash install.sh reinstall    # re-download and rewrite everything
#   sudo bash install.sh uninstall    # remove the software, keep data
#   sudo bash install.sh reset-password  # a new admin password, printed; data kept
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
# Offline / air-gapped install (no internet on the target):
#   BIRDNET_BINARY_TARBALL=/path/to/birdnet-behavior-<ver>-<target>.tar.gz \
#     BIRDNET_LOCAL_MODEL=/path/to/birdnet-v24.onnx sudo -E bash install.sh
#   Installs the binary from a tarball you already downloaded (skips the GitHub
#   fetch + checksum round-trip — you vouch for the local file), and the
#   classifier from a model file you already downloaded, which IS checked: it
#   must have the sha256 and size this release pins, exactly as a download must.
#   BIRDNET_SKIP_MODEL=1 instead skips the model altogether (~85 MB for BirdNET
#   V2.4, ~555 MB for the V3.0 preview): place the files in <data>/models and
#   restart afterwards.
#
# Exit status: 0 installed; 75 installed but the classifier could not be
# installed, so the station is NOT detecting (the summary says how to fix it);
# anything else, the install stopped.
#
# Environment overrides (all optional):
#   BIRDNET_BINARY_TARBALL=PATH  install the binary from a local tarball (offline)
#   BIRDNET_SKIP_MODEL=1         do not download the model (stage it out-of-band)
#   BIRDNET_LOCAL_MODEL=PATH     install the classifier from this file, as --local-model
#   BIRDNET_MODEL_RETRY_WAITS="30 90"  seconds between attempts at the model download
#   BIRDNET_CLASSIFIER=ID        birdnet-v24 (default) or birdnet-v3, as --classifier
#                                (the flag wins; an existing station keeps its own)
#   BIRDNET_NONINTERACTIVE=1     never prompt; take env/config/defaults
#   BIRDNET_LISTEN=HOST:PORT     dashboard bind address (default 0.0.0.0:8502)
#   BIRDNET_SKIP_GLIBC_CHECK=1   bypass the glibc floor check (built from source)
#
# Without systemd (a container, chroot, or staged image) the script still lays
# down the binary, config, and unit file, then tells you how to enable it on a
# real host — it does not abort.
#
# Requirements: curl or wget, tar, sha256sum (systemd recommended, not required)
