<h1 align="center">BirdNet-Behavior</h1>
<p align="center">Real-time acoustic bird classification with behavioral analytics — written in Rust, runs on a Raspberry Pi</p>

<p align="center">
  <a href="https://creativecommons.org/licenses/by-nc-sa/4.0/"><img src="https://img.shields.io/badge/License-CC%20BY--NC--SA%204.0-lightgrey.svg" alt="License"></a>
  <img src="https://img.shields.io/badge/Rust-1.88%2B-orange" alt="MSRV">
  <img src="https://img.shields.io/badge/platform-aarch64%20%7C%20armv7%20%7C%20x86__64-blue" alt="Platforms">
  <img src="https://img.shields.io/badge/clippy-pedantic%20%2B%20nursery-green" alt="Clippy">
</p>

> [!IMPORTANT]
> BirdNet-Behavior is licensed **CC BY-NC-SA 4.0** — the same terms as the upstream BirdNET model and BirdNET-Pi.
> **You may not use this project to build a commercial product.** See [LICENSE](LICENSE) for details.

---

**[Quick Install](#installation)** · **[Requirements](#requirements)** · **[First Steps](#first-steps)** · **[What's New](#new-features)** · **[Migrate from BirdNET-Pi](#birdnet-pi-migration)** · **[Troubleshooting](#troubleshooting)**

---

## Table of Contents

- [What is BirdNet-Behavior?](#what-is-birdnet-behavior)
- [Requirements](#requirements)
- [Installation](#installation)
- [First Steps](#first-steps)
- [Features](#features)
- [New Features](#new-features)
- [Configuration](#configuration)
- [Web UI](#web-ui)
- [BirdNET-Pi Migration](#birdnet-pi-migration)
- [Building from Source](#building-from-source)
- [Troubleshooting](#troubleshooting)
- [Architecture](#architecture)
- [Credits & Attribution](#credits--attribution)
- [License](#license)
- [Related Projects](#related-projects)
- [Documentation](#documentation)

---

## What is BirdNet-Behavior?

BirdNet-Behavior is a ground-up Rust rewrite of [BirdNET-Pi](https://github.com/mcguirepr89/BirdNET-Pi). It runs on a Raspberry Pi, listens to your microphone or RTSP camera, identifies birds in real time using the BirdNET+ neural network, and serves a web dashboard you open in any browser.

It ships as a **single static binary** — no Python, no pip, no virtualenv. Drop it on a Pi and run it.

| | BirdNET-Pi (Python) | BirdNet-Behavior (Rust) |
|---|---|---|
| Memory | 400–600 MB | ~20–50 MB |
| Cold start | 5–15 s | < 1 s |
| Dependencies | pip + venv + system libs | None |
| Upgrade | pip breakage, virtualenv rot | `scp` one file |
| Concurrency | GIL-constrained | Lock-free parallel audio |

A Raspberry Pi 4 has 2–4 GB RAM. BirdNet-Behavior uses roughly 2–5% of that, leaving the rest free for other tasks.

---

## Requirements

**Hardware:**

| Platform | Status |
|---|---|
| Raspberry Pi 5 | Recommended |
| Raspberry Pi 4B / 400 | Fully supported |
| Raspberry Pi 3B+ | Supported (armv7, 32-bit OS) |
| Raspberry Pi Zero 2W | Untested — may work on 64-bit OS |
| Any x86_64 Linux | Fully supported |

**Storage:** ~1.5 GB free — 541 MB for the BirdNET+ model, the rest for recordings and database.

**Audio input** (one of):
- USB microphone or USB sound card — `arecord` (from `alsa-utils`) used for capture
- IP camera or any RTSP stream — `ffmpeg` used for capture

**No other runtime dependencies.** The binary is statically linked.

---

## Installation

```bash
curl -fsSL https://raw.githubusercontent.com/tomtom215/BirdNet-Behavior/main/install.sh | sudo bash
```

The installer handles everything:

1. Detects your architecture (aarch64 / armv7 / x86\_64)
2. Downloads the pre-built binary from the latest GitHub Release
3. Downloads the BirdNET+ V3.0 model (~541 MB) from Zenodo
4. Creates `/etc/birdnet/birdnet.conf`, `~/BirdNet-Behavior/recordings/`, and `~/BirdNet-Behavior/models/`
5. Installs and enables a systemd service (`birdnet-behavior.service`)
6. Auto-detects your ALSA microphone and writes it into the config
7. Starts the service immediately if a microphone was found

**Install a specific version:**

```bash
VERSION=0.2.0 bash <(curl -fsSL https://raw.githubusercontent.com/tomtom215/BirdNet-Behavior/main/install.sh)
```

**Uninstall** (your recordings and database are preserved):

```bash
curl -fsSL https://raw.githubusercontent.com/tomtom215/BirdNet-Behavior/main/install.sh | sudo bash -s uninstall
```

---

## First Steps

After the installer finishes, open the web dashboard:

```
http://<your-pi-ip>:8502
```

> Not sure of your Pi's IP? Run `hostname -I` on the Pi, or check your router's device list.

**Recommended first visit:**

1. Go to **`/admin/settings`** — set your latitude/longitude for location-based species filtering, confirm your audio source, and optionally name your station
2. Return to **`/`** — the dashboard shows live detections as they come in; each card has the species name, confidence, a waveform player, and a Wikipedia image
3. Visit **`/species`** to browse all species detected so far, sorted by detection count

If the service is not yet running (no audio device was auto-detected during install), edit the config and start it:

```bash
sudo nano /etc/birdnet/birdnet.conf
# Set REC_CARD=plughw:1,0  or  RTSP_STREAM=rtsp://camera.local:554/stream
sudo systemctl start birdnet-behavior
```

---

## Features

Everything BirdNET-Pi does:

| Feature | Notes |
|---|---|
| Real-time detection | Microphone or RTSP stream |
| BirdNET+ V3.0 model | Same accuracy as upstream |
| SQLite detection database | Full history, fast queries |
| Web dashboard | Live feed, species list, stats |
| Per-species pages | Hourly activity chart, 14-day trend, Wikipedia image |
| Apprise push notifications | Telegram, Slack, Discord, 80+ channels |
| BirdWeather uploads | Station API compatible |
| Email alerts | SMTP/STARTTLS, per-species cooldown |
| CSV / JSON export | Full detection history |
| Species image cache | Wikipedia / Wikimedia |
| Admin settings panel | All config via web UI, no command line needed |
| Database backup / restore | Download from web UI |
| HTTP Basic Auth | Caddy / `CADDY_PWD` compatible |
| Audio file serving | Inline playback in browser |

---

## New Features

### Behavioral Analytics

Powered by [duckdb-behavioral](https://github.com/tomtom215/duckdb-behavioral). Requires the `--features analytics` build or the pre-built analytics binary.

- **Activity sessions** — identifies when birds are actively vocalizing vs. just passing through
- **Residents vs. migrants** — species return-interval analysis flags regulars, seasonal visitors, and rarities
- **Dawn chorus validation** — checks that species arrive in the expected sequence
- **Species correlations** — detects whether species A reliably predicts species B within a time window
- **Migration phenology** — weekly relative abundance index, arrival/departure date ranges, year-over-year trends

**Analytics dashboards:**
- Hour × day-of-week heatmap (SVG, no JavaScript)
- Species co-occurrence correlation matrix
- Companion species ("frequently detected together") lookup
- Hourly detection bar charts with dawn/dusk highlights

### Dual-Database Architecture

**SQLite** handles all real-time writes (<1 ms inserts, WAL mode). **DuckDB** handles analytics queries against the full history — columnar storage, vectorized execution, sub-second aggregations over millions of rows.

### IoT / Home Automation

- **MQTT publishing** — pure Rust, no external broker library; publishes detections as JSON to `{prefix}/detection/{species}`
- **Home Assistant auto-discovery** — `--mqtt-ha-discovery` creates sensors in HA automatically (last species, confidence, station status, daily count) — no `configuration.yaml` edits
- Compatible with Mosquitto, Node-RED, and any MQTT 3.1.1 broker
- Full config: `--mqtt-host`, `--mqtt-port`, `--mqtt-username`, `--mqtt-password`, `--mqtt-topic-prefix`, `--mqtt-retain`

### Audio Quality Pre-Filtering

A four-stage pipeline runs before ML inference: SNR estimation, spectral flatness (Wiener entropy), adaptive noise-floor tracking, and rain/wind detection via first-order IIR filters. Enable with `--quality-filter` to skip low-SNR recordings and reduce false positives.

### Observability

- Prometheus metrics at `/api/v2/metrics`
- JSON health check at `/api/v2/health`

### Admin Improvements

- Live log viewer (SSE, level filtering, pause, auto-scroll)
- System monitor (CPU, memory, temperature, uptime, disk)
- Species filter tester — preview include/exclude lists before applying
- One-click binary auto-update via GitHub Releases
- SMTP test connection from settings page

### Audio & Spectrogram

- Custom audio player with spectrogram visualization and playhead overlay
- Live spectrogram WebSocket stream (real-time mel spectrogram as you listen)
- tmpfs transient audio support — reduces SD card writes on Raspberry Pi

---

## Configuration

Settings are read in this priority order (highest wins):

```
CLI flags  >  environment variables  >  settings DB  >  /etc/birdnet/birdnet.conf  >  defaults
```

The easiest way to configure BirdNet-Behavior is through the **web UI at `/admin/settings`** — changes are stored in the database and survive binary upgrades.

For scripted or headless setups, use the config file at `/etc/birdnet/birdnet.conf` or environment variables:

```bash
# Override any setting with an env var prefixed BIRDNET_
BIRDNET_BIRDWEATHER_TOKEN=abc123 systemctl restart birdnet-behavior
```

### Key Settings

| Setting | Description | Default |
|---|---|---|
| `confidence_threshold` | Minimum confidence to record a detection | `0.70` |
| `sensitivity` | Detection sensitivity (0.5–1.5) | `1.0` |
| `alsa_device` | ALSA device for microphone input (`plughw:1,0`) | — |
| `rtsp_url` | RTSP stream URL | — |
| `latitude` / `longitude` | Station coordinates (species filtering, BirdWeather) | — |
| `recording_days` | Days to retain audio files before purging | `30` |
| `apprise_url` | Apprise server URL for push notifications | — |
| `birdweather_token` | BirdWeather station token | — |
| `email_smtp_host` | SMTP server for email alerts | — |
| `email_to` | Email alert recipient | — |
| `mqtt_host` | MQTT broker hostname (enables MQTT publishing) | — |
| `mqtt_port` | MQTT broker port | `1883` |
| `mqtt_topic_prefix` | MQTT topic prefix | `birdnet` |
| `mqtt_ha_discovery` | Publish Home Assistant auto-discovery messages | `false` |
| `quality_filter` | Enable audio quality pre-filtering | `false` |
| `quality_min_snr_db` | Minimum SNR to accept a recording | `3.0` |

---

## Web UI

### Dashboard & Species

| URL | Description |
|---|---|
| `/` | Live dashboard — detection feed with audio player, top species, activity stats |
| `/species` | All detected species with live search, detection counts, and confidence |
| `/species/detail?name=…` | Per-species page: hourly chart, 14-day trend, Wikipedia image, recent detections |
| `/heatmap` | Hour × day-of-week SVG heatmap + hourly bar chart |
| `/correlation` | Species co-occurrence pairs and companion species lookup |
| `/analytics` | Behavioral analytics dashboard (requires `--analytics-db`) |
| `/player/{filename}` | Audio player with spectrogram visualization |
| `/live` | Live audio stream |

### Admin

| URL | Description |
|---|---|
| `/admin/settings` | All settings: audio, location, detection, notifications, email, MQTT, species, system |
| `/admin/migrate` | BirdNET-Pi database migration |
| `/admin/system` | CPU / memory / temperature / disk |
| `/admin/system/backups` | List, download, and delete database backups |
| `/admin/system/logs/page` | Live log viewer with level filtering |
| `/admin/notifications` | Notification history (all channels) |
| `/admin/species/test` | Preview species include/exclude filter before saving |
| `/admin/update/check` | Check for and apply binary updates |

### API

| URL | Description |
|---|---|
| `/api/v2/metrics` | Prometheus metrics |
| `/api/v2/health` | JSON health check |

---

## BirdNET-Pi Migration

Safe, non-destructive import from an existing BirdNET-Pi installation. The source database is opened read-only and never modified.

```
1. Stop BirdNET-Pi        sudo systemctl stop birdnet_*
2. Note your database     it's at ~/BirdNET-Pi/BirdDB.txt
3. Open the migrate page  http://<your-pi>:8502/admin/migrate
4. Upload or enter path   drag-and-drop BirdDB.txt, or enter the file path if it's on the same machine
5. Review the preview     top 20 species, date range, data quality report (nulls, duplicates)
6. Import                 click Import — transaction-backed, fails cleanly on any error
7. Verify                 per-species count comparison shown after import
```

Duplicate rows (same date/time/species) are silently skipped, so re-running an import is safe.

---

## Building from Source

**Prerequisites:** [Rust 1.88+](https://rustup.rs), `git`

```bash
git clone https://github.com/tomtom215/BirdNet-Behavior.git
cd BirdNet-Behavior
```

Choose your build:

```bash
# Local testing (fast compile, unoptimized)
cargo build

# Deploy to Pi or server (optimized, ~3–5 min on a laptop)
cargo build --release

# With behavioral analytics (pulls in DuckDB C++ — ~7 min first build)
cargo build --release --features analytics

# Cross-compile for Raspberry Pi from x86_64 (requires `cross`)
cross build --release --target aarch64-unknown-linux-gnu
```

```bash
# Run the full test suite
cargo test --workspace

# Lint (pedantic + nursery, warnings denied)
cargo clippy --workspace --all-targets
```

**MSRV:** Rust 1.88 (edition 2024)

> **Note on build times:** The `--features analytics` build compiles DuckDB from source the first time (~7 minutes). Subsequent builds are cached. The base binary (no analytics) builds in under a minute on a modern laptop.

---

## Troubleshooting

**Service won't start:**
```bash
sudo journalctl -u birdnet-behavior -f
# Common cause: no audio source set in /etc/birdnet/birdnet.conf
```

**Web UI not reachable:**
```bash
# Check the service is running
sudo systemctl status birdnet-behavior

# Check port 8502 is listening
ss -tlnp | grep 8502

# Check firewall (Raspberry Pi OS doesn't restrict by default, but Ubuntu does)
sudo ufw allow 8502/tcp
```

**No detections appearing:**
```bash
# Verify your microphone is visible to ALSA
arecord -l

# Test a 5-second recording
arecord -D plughw:1,0 -f S16_LE -r 48000 -c 1 /tmp/test.wav && aplay /tmp/test.wav

# Then update your config and restart
sudo nano /etc/birdnet/birdnet.conf   # set REC_CARD=plughw:X,Y
sudo systemctl restart birdnet-behavior
```

**Wrong microphone selected:**
```bash
arecord -l          # list all capture devices
# Update REC_CARD in /etc/birdnet/birdnet.conf to match
sudo systemctl restart birdnet-behavior
```

**Model not found after install:**
```bash
ls ~/BirdNet-Behavior/models/
# If empty, the Zenodo download failed. Re-run the installer:
curl -fsSL https://raw.githubusercontent.com/tomtom215/BirdNet-Behavior/main/install.sh | sudo bash
# The installer skips steps that already completed, including the binary download.
```

**View live logs:**
```bash
sudo journalctl -u birdnet-behavior -f
# Or use the web UI at /admin/system/logs/page
```

---

## Architecture

BirdNet-Behavior is a single binary built from 8 Rust workspace crates. There are no shared libraries to install, no runtime dependencies, and no interpreter.

```
birdnet-behavior (single binary)
├── birdnet-core          Audio capture, decode, resample, mel spectrogram, ML inference
├── birdnet-db            SQLite (OLTP) + DuckDB (OLAP), migrations, resilience, backup
├── birdnet-web           axum web server, REST API, WebSocket, HTMX templates, audio player
├── birdnet-integrations  BirdWeather, Apprise, email, Wikipedia images, MQTT, auto-update
├── birdnet-behavioral    DuckDB behavioral analytics (feature: analytics)
├── birdnet-timeseries    Activity, diversity, trend, peak, gap, and session analytics
├── birdnet-migrate       BirdNET-Pi schema detection, validation, and import
└── birdnet-scheduler     Solar calculations, recording window scheduling
```

The audio pipeline runs synchronously in blocking threads (`tokio::task::spawn_blocking`). The web server runs in the async Tokio runtime. They communicate through channels — the audio pipeline pushes detection events; the web server broadcasts them to WebSocket clients and writes them to the database.

See [`docs/architecture/`](docs/architecture/) for the full design documents.

---

## Credits & Attribution

BirdNet-Behavior builds on these projects:

- **[BirdNET](https://github.com/kahst/BirdNET-Analyzer)** — ML model by the K. Lisa Yang Center for Conservation Bioacoustics, Cornell Lab of Ornithology
- **[BirdNET-Pi](https://github.com/mcguirepr89/BirdNET-Pi)** — original Raspberry Pi implementation by [Patrick McGuire](https://github.com/mcguirepr89)
- **[BirdNET-Pi fork](https://github.com/Nachtzuster/BirdNET-Pi)** — maintained fork by [Nachtzuster](https://github.com/Nachtzuster) (Bookworm support, backup/restore, many fixes)
- **[duckdb-behavioral](https://github.com/tomtom215/duckdb-behavioral)** — behavioral analytics library by [tomtom215](https://github.com/tomtom215)

BirdNet-Behavior is a **clean rewrite**, not a fork. Architecture and feature decisions were informed by [tomtom215's BirdNET-Pi fork](https://github.com/tomtom215/BirdNET-Pi).

---

## License

Licensed under [CC BY-NC-SA 4.0](https://creativecommons.org/licenses/by-nc-sa/4.0/), matching the upstream BirdNET and BirdNET-Pi projects.

See [LICENSE](LICENSE) and [LICENSE-UPSTREAM](LICENSE-UPSTREAM) for full attribution and terms.

---

## Related Projects

| Repository | Description |
|---|---|
| [duckdb-behavioral](https://github.com/tomtom215/duckdb-behavioral) | ClickHouse-inspired behavioral analytics for DuckDB |
| [quack-rs](https://github.com/tomtom215/quack-rs) | SDK for building DuckDB extensions in Rust |
| [mallardmetrics](https://github.com/tomtom215/mallardmetrics) | Single-binary web analytics (axum + DuckDB) |
| [LyreBirdAudio](https://github.com/tomtom215/LyreBirdAudio) | RTSP audio streaming |

---

## Documentation

Detailed architecture and design documents live in [`docs/`](docs/):

| Document | Contents |
|---|---|
| [`docs/RUST_ARCHITECTURE_PLAN.md`](docs/RUST_ARCHITECTURE_PLAN.md) | Full architecture overview and module index |
| [`docs/architecture/01-motivation.md`](docs/architecture/01-motivation.md) | Why Rust, design philosophy |
| [`docs/architecture/02-architecture.md`](docs/architecture/02-architecture.md) | Single-binary design, workspace layout |
| [`docs/architecture/10-deployment.md`](docs/architecture/10-deployment.md) | Cross-compilation, CI/CD, systemd |
| [`docs/architecture/11-migration.md`](docs/architecture/11-migration.md) | BirdNET-Pi migration design |
| [`docs/architecture/13-implementation-status.md`](docs/architecture/13-implementation-status.md) | Current implementation status and test coverage |
