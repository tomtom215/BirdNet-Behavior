<h1 align="center">
  BirdNet-Behavior
</h1>
<p align="center">
Real-time acoustic bird classification with behavioral analytics, written in Rust
</p>

<p align="center">
  <a href="https://creativecommons.org/licenses/by-nc-sa/4.0/"><img src="https://img.shields.io/badge/License-CC%20BY--NC--SA%204.0-lightgrey.svg"></a>
  <img src="https://img.shields.io/badge/Rust-1.85%2B-orange">
  <img src="https://img.shields.io/badge/platform-aarch64%20%7C%20x86__64-blue">
</p>

<h2 align="center"><a href="LICENSE">Review the license!</a></h2>
<h3 align="center">You may not use BirdNet-Behavior to develop a commercial product!</h3>

---

## Table of Contents

- [About](#about)
- [Feature Parity with BirdNET-Pi](#feature-parity-with-birdnet-pi)
- [New Features](#new-features)
- [Architecture](#architecture)
- [Web UI](#web-ui)
- [Admin Panel](#admin-panel)
- [BirdNET-Pi Migration](#birdnet-pi-migration)
- [Requirements](#requirements)
- [Installation](#installation)
- [Building from Source](#building-from-source)
- [Configuration](#configuration)
- [Credits & Attribution](#credits--attribution)
- [License](#license)

---

## About

BirdNet-Behavior is a ground-up Rust rewrite of [BirdNET-Pi](https://github.com/mcguirepr89/BirdNET-Pi), designed for resource-constrained and solar-powered bird monitoring stations. It ships as a **single binary** with no Python, no pip, no virtualenv — just `scp` it to your Pi and run.

| Metric | BirdNET-Pi (Python) | BirdNet-Behavior (Rust) |
|--------|---------------------|-------------------------|
| Memory (RSS) | 400–600 MB | ~20–50 MB |
| Cold start | 5–15 s | < 1 s |
| Dependencies | pip + venv + system libs | None (single static binary) |
| Upgrade risk | pip breakage, virtualenv rot | `scp` new binary |
| Concurrency | GIL-constrained | Lock-free parallel audio |

---

## Feature Parity with BirdNET-Pi

✅ Real-time bird detection from microphone or RTSP stream
✅ BirdNET ONNX model inference (same ML model, same accuracy)
✅ SQLite detection database with full history
✅ Web dashboard (live detections, species list, stats)
✅ Species detail pages with hourly activity charts
✅ Apprise push notifications (Telegram, Slack, Discord, 80+ channels)
✅ BirdWeather station uploads
✅ Email alerts (SMTP, STARTTLS/TLS, per-species cooldown)
✅ Export detections as CSV or JSON
✅ Species image caching (Wikipedia/Wikimedia)
✅ Admin settings panel
✅ Database backup and restore
✅ HTTP Basic Auth (Caddy/CADDY_PWD compatible)
✅ Audio file serving and inline playback

---

## New Features

**Behavioral Analytics** powered by [duckdb-behavioral](https://github.com/tomtom215/duckdb-behavioral):
- Activity sessionization — when birds are actively vocalizing vs. passing through
- Species retention analysis — residents vs. migrants vs. rarities
- Dawn chorus funnel analysis — validate species ordering patterns
- Sequence pattern matching — does species A predict species B?
- Real-time next-species prediction

**Dual-Database Architecture:**
- **SQLite** for real-time OLTP (detection writes, live queries, <1 ms inserts)
- **DuckDB** for columnar OLAP analytics (trends, aggregations, behavioral patterns)

**Interactive Analytics Dashboards:**
- Hour × day-of-week activity heatmap (SVG, no JS required)
- Species co-occurrence correlation matrix
- Species-pair companion analysis ("frequently seen together")
- Temporal co-occurrence (within N minutes)
- Hourly detection bar charts (dawn/dusk highlighted)

**BirdNET-Pi Migration:**
- Web UI file upload — drag and drop your `BirdDB.txt` or `birds.db`
- Pre-flight validation with data quality report (null dates, invalid confidence, duplicates)
- Species-level preview before import (top 20 species, date range)
- Zero-modification guarantee — source file is **never touched**
- Post-migration verification (per-species count comparison)
- Server-path mode for on-disk migrations

**IoT / Home Automation:**
- MQTT 3.1.1 publisher — pure Rust, no external library, detections published as JSON to `{prefix}/detection/{species}`
- Compatible with Home Assistant, Mosquitto, Node-RED, any MQTT 3.1.1 broker
- Optional RETAIN flag for Home Assistant sensor persistence
- Full CLI/env var configuration: `--mqtt-host`, `--mqtt-port`, `--mqtt-username`, `--mqtt-password`, `--mqtt-topic-prefix`, `--mqtt-retain`

**Audio Quality Pre-Filtering:**
- Four-stage pipeline: SNR estimation, spectral flatness (Wiener entropy), adaptive noise-floor tracking, rain/wind detection
- Purely time-domain rain/wind detection via first-order IIR filters — O(N), suitable for Raspberry Pi
- Optional pre-ML-inference gate: `--quality-filter`, `--quality-min-snr-db`

**Migration Phenology Analytics:**
- Weekly relative abundance index normalized to peak week
- Migration window timing via `percentile_cont` (arrival/departure date ranges)
- Inter-annual trend analysis with year-over-year detection change (DuckDB)
- Species richness and effort-corrected abundance queries

**Performance Benchmarks:**
- Criterion benchmark suite for audio pipeline (mel spectrogram, SNR, rain detection, noise floor)
- Criterion benchmark suite for database queries (insert, batch transaction, aggregation, search)
- HTML benchmark reports for regression tracking

**Observability:**
- Prometheus metrics endpoint (`/api/v2/metrics`)
- Health check endpoint (`/api/v2/health`)

**Admin Improvements:**
- SSE live log viewer with level filtering, pause, auto-scroll
- Database backup management (list, download, delete)
- System resource monitoring (CPU, memory, temperature, uptime)
- Email alert settings with SMTP test connection
- Species filter tester/preview (validate include/exclude lists before applying)
- Binary auto-update (check GitHub Releases + one-click atomic update)

**Audio & Spectrogram:**
- Custom audio player with spectrogram visualization and playhead overlay
- Live spectrogram WebSocket push (real-time mel spectrogram streaming)
- tmpfs transient audio support (reduce SD card wear on Raspberry Pi)

---

## Architecture

```
birdnet-behavior (single binary)
├── Core Engine  (birdnet-core)
│   ├── Audio Capture   — mic (arecord) or RTSP (ffmpeg) subprocess management
│   ├── Audio Decode    — symphonia (WAV/FLAC/MP3/OGG, pure Rust)
│   ├── Resample        — rubato (high-quality polynomial resampler)
│   ├── Mel Spectrogram — pure Rust FFT + 128 mel bands (librosa-compatible)
│   ├── Audio Quality   — SNR, spectral flatness, noise-floor tracker, rain/wind IIR
│   ├── ML Inference    — tract-onnx ONNX runtime (pure Rust, no C++)
│   └── Detection Daemon — file watcher → quality gate → inference → event channel
│
├── Data Layer   (birdnet-db)
│   ├── SQLite OLTP    — WAL mode, CRUD, aggregation queries, migrations
│   ├── DuckDB OLAP    — behavioral analytics (optional feature flag)
│   ├── Settings       — persistent key-value store backed by SQLite
│   ├── Notifications  — alert log with channel/status/species history
│   └── Resilience     — WAL, backup, integrity checks, auto-recovery
│
├── Web Server   (birdnet-web)
│   ├── REST API       — /api/v2/* (detections, species, analytics, export)
│   ├── Metrics        — /api/v2/metrics (Prometheus), /api/v2/health
│   ├── WebSocket      — live detection streaming (JSON events)
│   ├── HTMX UI        — server-rendered dark-theme dashboard
│   ├── Admin Panel    — settings, migration, system, logs, backups
│   └── Static files   — embedded HTMX JS (air-gapped compatible)
│
├── Integrations (birdnet-integrations)
│   ├── Email          — SMTP via lettre + rustls (no OpenSSL)
│   ├── Apprise        — push notifications (80+ channels), cooldown tracking
│   ├── BirdWeather    — station detection uploads with retry backoff
│   ├── Species Images — Wikipedia/Wikimedia image caching
│   ├── Auto-Update   — GitHub Releases check + atomic binary replace
│   └── MQTT           — pure-Rust MQTT 3.1.1 (Home Assistant, Node-RED, Mosquitto)
│
├── Behavioral   (birdnet-behavioral, feature: analytics)
│   ├── Sessionize     — gap-based activity window detection
│   ├── Retention      — species return interval analysis
│   ├── Funnels        — dawn chorus sequence validation
│   ├── Predictions    — next-species likelihood
│   └── Phenology      — migration timing, weekly abundance index, inter-annual trends
│
├── Migration    (birdnet-migrate)
│   ├── Validators     — schema detection, data quality checks
│   ├── Importers      — BirdNET-Pi SQLite → BirdNet-Behavior SQLite
│   └── Reports        — pre/post migration species-level statistics
│
└── Time Series  (birdnet-timeseries)
    └── Activity, diversity, trend, peak, gap, session analytics
```

---

## Web UI

| URL | Description |
|-----|-------------|
| `/` | Live dashboard — detections table with audio player, top species, stats |
| `/species` | Species list with live search, detection counts, confidence |
| `/species/detail?name=...` | Per-species page with hourly chart, 14-day trend, Wikipedia image |
| `/heatmap` | Hour × day-of-week SVG heatmap + hourly bar chart |
| `/correlation` | Species co-occurrence pairs and companion species lookup |
| `/analytics` | Behavioral analytics dashboard (requires `--analytics-db`) |
| `/health` | System health page |
| `/player/{filename}` | Custom audio player with spectrogram visualization |
| `/live` | Live audio stream page |
| `/api/v2/metrics` | Prometheus metrics endpoint |
| `/api/v2/health` | Health check endpoint (JSON) |

---

## Admin Panel

| URL | Description |
|-----|-------------|
| `/admin/settings` | All settings: audio, location, detection, notifications, email, species, system |
| `/admin/migrate` | BirdNET-Pi migration — file upload or server path |
| `/admin/system` | CPU/memory/temperature, disk usage, recording stats |
| `/admin/system/backups` | List, download, and delete database backups |
| `/admin/system/logs/page` | Live SSE log viewer with filtering and pause |
| `/admin/notifications` | Notification history log (all channels) |
| `/admin/species/test` | Species filter tester/preview |
| `/admin/update/check` | Check for binary updates (GitHub Releases) |

---

## BirdNET-Pi Migration

Safe, non-destructive import from an existing BirdNET-Pi installation:

```
1. Shut down BirdNET-Pi:    sudo systemctl stop birdnet_*
2. Copy the database:       cp ~/BirdNET-Pi/BirdDB.txt /tmp/birds.db
3. Open BirdNet-Behavior:   http://your-pi:8502/admin/migrate
4. Upload or enter path:    Upload BirdDB.txt or enter the server path
5. Preview & import:        Review the species report, click Import
6. Verify:                  Post-migration per-species count comparison shown
```

**Guarantees:**
- Source database is **opened read-only** and **never modified**
- All validation runs before any data is written to the destination
- Duplicate rows (same date/time/species) are skipped, not duplicated
- Transaction-backed import — fails cleanly without partial state

---

## Requirements

- Raspberry Pi 5, 4B, or 400 (64-bit required) — or any x86_64 Linux
- A USB microphone or sound card (or RTSP IP camera with audio)
- That's it. No Python. No pip. No system dependencies at runtime.

**For audio capture:**
- `arecord` (from `alsa-utils`) for microphone capture
- `ffmpeg` for RTSP stream capture

---

## Installation

```bash
# Download the latest release for your platform
curl -L https://github.com/tomtom215/BirdNet-Behavior/releases/latest/download/birdnet-behavior-aarch64 \
  -o /usr/local/bin/birdnet-behavior
chmod +x /usr/local/bin/birdnet-behavior

# Run with minimal config
birdnet-behavior \
  --model /path/to/BirdNET_GLOBAL_6K_V2.4_Model_FP32.tflite \
  --labels /path/to/labels.txt \
  --watch-dir /home/pi/BirdSongs/Extracted/By_Date

# Web-only mode (no detection, just the web UI against an existing DB)
birdnet-behavior --web-only --listen 0.0.0.0:8502
```

**Systemd service:**

```ini
[Unit]
Description=BirdNet-Behavior
After=network.target sound.target

[Service]
ExecStart=/usr/local/bin/birdnet-behavior \
  --model /etc/birdnet/BirdNET_GLOBAL_6K_V2.4_Model_FP32.tflite \
  --labels /etc/birdnet/labels.txt \
  --watch-dir /home/pi/BirdSongs/Extracted/By_Date \
  --listen 0.0.0.0:8502
Restart=on-failure
User=pi

[Install]
WantedBy=multi-user.target
```

---

## Building from Source

```bash
# Clone
git clone https://github.com/tomtom215/BirdNet-Behavior.git
cd BirdNet-Behavior

# Build (debug — fast compile, slow runtime)
cargo build

# Build (release, optimized for Pi deployment)
cargo build --release

# Build with DuckDB behavioral analytics (~7 min first build due to bundled C++)
cargo build --release --features analytics

# Cross-compile for Raspberry Pi (requires cross)
cross build --release --target aarch64-unknown-linux-gnu

# Run tests
cargo test --workspace

# Lint
cargo clippy --workspace --all-targets
```

### MSRV

Rust 1.85+ (edition 2024)

---

## Configuration

Settings can be provided via:

1. **Web UI** — `/admin/settings` (stored in SQLite `settings` table, survives upgrades)
2. **CLI flags** — `birdnet-behavior --help` for full list
3. **Config file** — INI-format at `/etc/birdnet/birdnet.conf` (BirdNET-Pi compatible)
4. **Environment variables** — `BIRDNET_APPRISE_URL`, `BIRDNET_BIRDWEATHER_TOKEN`, etc.

Priority order: CLI flags > environment variables > settings DB > config file > defaults.

### Key Settings

| Setting | Description | Default |
|---------|-------------|---------|
| `confidence_threshold` | Minimum confidence to save detection | `0.70` |
| `sensitivity` | Detection sensitivity (0.5–1.5) | `1.0` |
| `alsa_device` | ALSA device for microphone capture | — |
| `rtsp_url` | RTSP audio stream URL | — |
| `apprise_url` | Apprise server URL for push notifications | — |
| `birdweather_token` | BirdWeather station token | — |
| `email_smtp_host` | SMTP server for email alerts | — |
| `email_to` | Email alert recipient | — |
| `latitude` / `longitude` | Station location (for BirdWeather) | — |
| `recording_days` | Days to keep audio files | `30` |
| `mqtt_host` | MQTT broker host (enables MQTT publishing) | — |
| `mqtt_port` | MQTT broker port | `1883` |
| `mqtt_topic_prefix` | MQTT topic prefix for detection events | `birdnet` |
| `quality_filter` | Enable audio quality pre-filtering | `false` |
| `quality_min_snr_db` | Minimum SNR threshold for quality filter | `3.0` |

---

## Credits & Attribution

BirdNet-Behavior is built on the shoulders of these excellent projects:

- **[BirdNET](https://github.com/kahst/BirdNET-Analyzer)** by [@kahst](https://github.com/kahst) — the ML framework and models for bird sound classification (K. Lisa Yang Center for Conservation Bioacoustics, Cornell Lab of Ornithology)
- **[BirdNET-Pi](https://github.com/mcguirepr89/BirdNET-Pi)** by [Patrick McGuire](https://github.com/mcguirepr89) — the original Raspberry Pi implementation
- **[BirdNET-Pi fork](https://github.com/Nachtzuster/BirdNET-Pi)** by [Nachtzuster](https://github.com/Nachtzuster) — maintained fork with Bookworm support, backup/restore, and many improvements
- **[duckdb-behavioral](https://github.com/tomtom215/duckdb-behavioral)** by [tomtom215](https://github.com/tomtom215) — ClickHouse-inspired behavioral analytics for DuckDB

This project is **not a fork** — it is a clean Rust rewrite. The architecture, design decisions, and feature analysis were informed by [tomtom215's BirdNET-Pi fork](https://github.com/tomtom215/BirdNET-Pi).

---

## License

BirdNet-Behavior is licensed under the [Creative Commons Attribution-NonCommercial-ShareAlike 4.0 International License](https://creativecommons.org/licenses/by-nc-sa/4.0/), matching the upstream BirdNET and BirdNET-Pi projects.

See [LICENSE](LICENSE) and [LICENSE-UPSTREAM](LICENSE-UPSTREAM) for full details.

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

Detailed architecture documentation lives in [`docs/`](docs/):

| Document | Contents |
|----------|----------|
| [`docs/RUST_ARCHITECTURE_PLAN.md`](docs/RUST_ARCHITECTURE_PLAN.md) | Architecture overview and module index |
| [`docs/architecture/01-motivation.md`](docs/architecture/01-motivation.md) | Why Rust, design philosophy |
| [`docs/architecture/02-architecture.md`](docs/architecture/02-architecture.md) | Single binary design, workspace layout |
| [`docs/architecture/13-implementation-status.md`](docs/architecture/13-implementation-status.md) | **Current implementation status and test coverage** |
| [`docs/architecture/11-migration.md`](docs/architecture/11-migration.md) | BirdNET-Pi migration design |
