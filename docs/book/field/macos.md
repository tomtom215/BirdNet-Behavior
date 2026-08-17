# Running BirdNet-Behavior on macOS (Apple Silicon)

> **Status: preview — pending hardware verification.** The workspace is
> process-based for audio capture and `ort` ships an `aarch64-apple-darwin`
> ONNX Runtime prebuilt, so BirdNet-Behavior is expected to build and run on
> Apple Silicon (M-series) Macs. The macOS-specific paths below (the
> avfoundation microphone source, the launchd service, the release build) have
> not yet been confirmed on real hardware or a `macos-14` CI run. Treat this as
> a setup guide to validate, not a support guarantee.

## Quick install

`install.sh` is OS-aware: on macOS it sets up a per-user launchd LaunchAgent
(not systemd), so run it **without `sudo`**:

```bash
curl -fsSL https://raw.githubusercontent.com/tomtom215/BirdNet-Behavior/main/install.sh | bash
```

Once a release publishes the `aarch64-apple-darwin` build, this downloads and
installs it, writes a starter config under `~/Library/Application Support/birdnet-behavior/`,
and writes a ready-to-load LaunchAgent. Until then it offers to `brew install`
the dependencies and prints the one-time source-build steps below. A Homebrew
formula is planned so this becomes `brew install`. Uninstall any time with
[`./uninstall.sh`](https://github.com/tomtom215/BirdNet-Behavior/blob/main/uninstall.sh)
(`--purge` to remove data too).

## What works the same as Linux

File/`--watch-dir` analysis, RTSP capture (via `ffmpeg`), the web UI, SQLite,
DuckDB analytics, and ML inference are all platform-independent — they shell
out to `ffmpeg`/`arecord` or use cross-platform crates.

## Prerequisites

```bash
# Rust toolchain
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
# Build + runtime deps (cmake builds the bundled libduckdb; ffmpeg captures audio)
brew install cmake ffmpeg
```

## Build from source

```bash
git clone https://github.com/tomtom215/BirdNet-Behavior.git
cd BirdNet-Behavior
cargo build --release --features analytics    # ONNX Runtime downloads on first build
```

The binary lands at `target/release/birdnet-behavior`.

### One-shot verification

[`packaging/macos/verify-macos.sh`](https://github.com/tomtom215/BirdNet-Behavior/blob/main/packaging/macos/verify-macos.sh)
runs the whole from-source check in one go — toolchain/dependency probe, release build,
`--doctor` preflight, a real `--web-only` boot with a `GET /` check, and
`avfoundation` device enumeration — then prints the manual steps (microphone/TCC
consent, the launchd LaunchAgent) that need a human at the machine:

```bash
bash packaging/macos/verify-macos.sh            # full build + boot + report
SKIP_BUILD=1 bash packaging/macos/verify-macos.sh   # reuse an existing build
```

It installs nothing and changes no system settings. Paste its output back when
reporting macOS results.

## Live microphone capture (avfoundation)

On macOS the microphone is captured through `ffmpeg`'s **avfoundation** input
rather than ALSA's `arecord` (which is Linux-only); the daemon selects this
automatically. List your audio input devices and their indices with:

```bash
ffmpeg -f avfoundation -list_devices true -i ""
```

Set the audio device index (default `0`) as the microphone `device` in your
config. The first capture will trigger a **Microphone access** prompt — grant
it under *System Settings → Privacy & Security → Microphone*. A headless
background service cannot obtain this consent, which is why the launchd unit
below is a per-user **LaunchAgent**, not a system daemon.

## Run

```bash
# UI only (screenshots / kiosk), no capture:
target/release/birdnet-behavior -c birdnet.conf --web-only --listen 127.0.0.1:8502
# Full station with mic + analytics:
target/release/birdnet-behavior -c birdnet.conf --analytics-db analytics.duckdb
```

## Run as a service (launchd)

A ready-to-edit LaunchAgent lives at
[`packaging/macos/com.tomtom215.birdnet-behavior.plist`](https://github.com/tomtom215/BirdNet-Behavior/blob/main/packaging/macos/com.tomtom215.birdnet-behavior.plist).
Copy it to `~/Library/LaunchAgents/`, edit the paths and station coordinates,
then `launchctl load -w` it. See the comments in the plist for details.

## What is *not* used on macOS

These Linux-only mechanisms are skipped or not applicable; none block the
build, but be aware:

- **tmpfs transient-audio mount** — uses Linux `mount`/`umount`; on macOS leave
  the tmpfs option off (a RAM disk via `hdiutil`/`diskutil` is the macOS
  equivalent and is not wired up).
- **systemd** service control, the watchdog, and `sd_notify` — already
  `cfg`-gated to Linux; the admin "restart service" controls are inert.
- **`/proc` system metrics** — some host metrics on the System page are
  Linux-specific.
- **`install.sh`** — a systemd + glibc Linux installer; do not run it on macOS.
  Use the build-from-source + launchd path above.

## Homebrew (planned)

A Homebrew formula/tap that pulls the `aarch64-apple-darwin` release tarball is
the intended end-user install path. It is not published yet; build from source
in the meantime. The release pipeline now builds the `aarch64-apple-darwin`
target so a formula has an artifact to reference once the target is verified.
