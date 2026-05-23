# Running BirdNet-Behavior on macOS (Apple Silicon)

> **Status: preview — pending hardware verification.** The workspace is
> process-based for audio capture and `ort` ships an `aarch64-apple-darwin`
> ONNX Runtime prebuilt, so BirdNet-Behavior is expected to build and run on
> Apple Silicon (M-series) Macs. The macOS-specific paths below (the
> avfoundation microphone source, the launchd service, the release build) have
> not yet been confirmed on real hardware or a `macos-14` CI run. Treat this as
> a setup guide to validate, not a support guarantee.

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
[`packaging/macos/com.tomtom215.birdnet-behavior.plist`](../packaging/macos/com.tomtom215.birdnet-behavior.plist).
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
