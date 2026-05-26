# Installation

There are two supported ways to install BirdNet-Behavior. **Docker is the fastest path** — two commands and you are watching detections. The bare-metal installer is ideal for a dedicated Raspberry Pi with no container runtime.

## Supported hardware & OS

| Platform | Status | Notes |
|---|---|---|
| Raspberry Pi 5 | ✅ Recommended | 64-bit Raspberry Pi OS **Trixie** |
| Raspberry Pi 4B / 400 | ✅ Fully supported | 64-bit Raspberry Pi OS **Trixie** |
| Any x86_64 Linux | ✅ Fully supported | glibc ≥ 2.39 (Debian 13 / Ubuntu 24.04 or newer) |
| Raspberry Pi OS **Bookworm** (glibc 2.36) | ⚠️ Docker only | The native binary needs glibc ≥ 2.39. Run the **Docker** image (it bundles its own runtime, so the host glibc does not matter) or upgrade to Trixie. |
| Pi 3 / Pi Zero 2 W on a **32-bit** OS (armv7) | ❌ | No prebuilt ONNX Runtime exists for armv7. These boards are 64-bit-capable — reflash with the 64-bit Pi OS. |

> **Runtime requirement: glibc ≥ 2.39.** The prebuilt binaries are built on
> Ubuntu 24.04 to match the baseline that pyke's ONNX Runtime requires, so they
> do **not** run on Raspberry Pi OS Bookworm / Debian 12 (glibc 2.36). `install.sh`
> checks this and refuses with a clear message rather than installing a binary
> that won't start. Bookworm users: use Docker. Each prebuilt binary also ships
> the DuckDB behavioral-analytics engine **built in and on by default** — the
> installer's systemd unit enables it automatically (for a manual run, pass
> `--analytics-db`).

- **Storage:** ~1.5 GB free (541 MB for the BirdNET+ model, the rest for recordings and database).
- **Audio input** — one of:
  - a USB microphone or USB sound card (`arecord` from `alsa-utils`), or
  - an IP camera or any RTSP stream (`ffmpeg`).

The BirdNET+ V3.0 model (~541 MB) and species labels are downloaded automatically from Zenodo on first run — you never pick, locate, or install a model yourself.

## Option 1 — Docker quick start (recommended)

One command. It asks two or three plain-English questions, auto-detects your USB mic, writes a minimal `.env`, and starts the container. No git clone, no editor, no hand-picking compose overlays.

```bash
bash <(curl -fsSL https://raw.githubusercontent.com/tomtom215/BirdNet-Behavior/main/quickstart.sh)
```

<details>
<summary>What the quick-start script does</summary>

1. Verifies Docker Engine + Compose are installed and usable (with clear remediation if not).
2. Checks disk space and that port 8502 is free.
3. Creates `~/birdnet-behavior/` for your `.env` and compose files.
4. Auto-detects your audio source — USB/ALSA card, PulseAudio/PipeWire, or falls back to asking for an RTSP URL.
5. Asks for your station latitude/longitude (with opt-in IP auto-detect via ipapi.co).
6. Asks whether to enable DuckDB behavioral analytics (default: no).
7. Writes a short `.env` with only your chosen values.
8. Starts the container with the matching compose overlay.
9. Streams logs so you can watch the one-time 541 MB model download.
10. Stops tailing as soon as the web server reports healthy, then prints the dashboard URL and your LAN IP.

</details>

See [Running with Docker](./docker.md) for the manual path, `docker run`, and audio-source overlays.

## Option 2 — Bare-metal installer (no Docker)

```bash
curl -fsSL https://raw.githubusercontent.com/tomtom215/BirdNet-Behavior/main/install.sh | sudo bash
```

The installer detects your architecture, downloads the pre-built binary and the BirdNET+ model, creates the config/recording/model directories, installs and enables a `systemd` service, auto-detects your ALSA microphone, and starts the service immediately.

```bash
# Install a specific version (defaults to latest). The `-s --` passes the
# argument through to the installer.
curl -fsSL https://raw.githubusercontent.com/tomtom215/BirdNet-Behavior/main/install.sh | sudo bash -s -- --version 0.5.1
```

> **Don't run the installer with `sudo bash <(curl ...)`.** Process substitution
> hands `bash` a file descriptor owned by your user; `sudo` closes it crossing
> to root, so the script disappears with `/dev/fd/63: No such file or directory`.
> The pipe forms above (`curl ... | sudo bash`) avoid that entirely. If you'd
> rather inspect the script first, download it and run it directly:
>
> ```bash
> curl -fsSL https://raw.githubusercontent.com/tomtom215/BirdNet-Behavior/main/install.sh -o install.sh
> sudo bash install.sh --version 0.5.1
> ```

Re-running the installer is an **upgrade**: it stops the service, swaps in the
new binary, and restarts it. Your configuration and the SQLite/DuckDB databases
are preserved, and schema migrations run automatically on the next start.

### Uninstalling

`uninstall.sh` ships next to the binary in every release tarball (and as a
standalone release asset). It is **safe by default** — it removes only the
software (the systemd service, the tmpfs mount unit, and the binary) and
**keeps your database, recordings, settings, and the downloaded model** unless
you opt in. It is idempotent (re-running is harmless) and refuses to touch
system directories.

```bash
sudo ./uninstall.sh                 # remove the software, keep all data
sudo ./uninstall.sh --dry-run       # preview the exact plan, change nothing
sudo ./uninstall.sh --purge         # remove EVERYTHING (data, settings, model)

# Or pick precisely what to delete:
sudo ./uninstall.sh --remove-db --remove-recordings
```

It auto-detects your real data directory from the installed config and service
files, so it removes exactly what was installed. On macOS it instead unloads
the launchd LaunchAgent; pass `--purge` to also remove the user data directory.

**Docker deployments** are torn down with Compose instead — from the directory
holding your `docker-compose.yml`:

```bash
docker compose down            # stop + remove the container, keep named volumes
docker compose down -v         # also remove volumes (deletes the database!)
```

## Building from source

**Prerequisites:** [Rust 1.95+](https://rustup.rs) and `git`.

```bash
git clone https://github.com/tomtom215/BirdNet-Behavior.git
cd BirdNet-Behavior

cargo build --release                              # optimized build (~3–5 min)
cargo build --release --features analytics         # + DuckDB analytics (~7 min first build)
cross build --release --target aarch64-unknown-linux-gnu   # cross-compile for a Pi
```

Once installed, head to [First Steps](./first-steps.md) to open the dashboard.
