<h1 align="center">BirdNet-Behavior</h1>
<p align="center">Real-time acoustic bird classification with behavioral analytics — written in Rust, runs on a Raspberry Pi.</p>

<p align="center">
  <a href="https://creativecommons.org/licenses/by-nc-sa/4.0/"><img src="https://img.shields.io/badge/License-CC%20BY--NC--SA%204.0-lightgrey.svg" alt="License"></a>
  <img src="https://img.shields.io/badge/Rust-1.95%2B-orange" alt="MSRV">
  <img src="https://img.shields.io/badge/platform-aarch64%20%7C%20x86__64-blue" alt="Platforms">
  <img src="https://img.shields.io/badge/clippy-pedantic%20%2B%20nursery-green" alt="Clippy">
  <img src="https://img.shields.io/badge/Docker-ghcr.io-2496ED" alt="Docker">
</p>

<p align="center">
  <strong>📖 <a href="https://tomtom215.github.io/BirdNet-Behavior/">Read the documentation</a></strong>
  &nbsp;·&nbsp; <a href="https://tomtom215.github.io/BirdNet-Behavior/getting-started/installation.html">Install</a>
  &nbsp;·&nbsp; <a href="https://tomtom215.github.io/BirdNet-Behavior/getting-started/docker.html">Docker</a>
  &nbsp;·&nbsp; <a href="https://tomtom215.github.io/BirdNet-Behavior/guide/dashboard.html">Field Guide</a>
  &nbsp;·&nbsp; <a href="https://tomtom215.github.io/BirdNet-Behavior/guides/troubleshooting.html">Troubleshooting</a>
</p>

> [!IMPORTANT]
> BirdNet-Behavior is licensed **CC BY-NC-SA 4.0** — the same terms as the upstream BirdNET model and BirdNET-Pi.
> **You may not use this project to build a commercial product.** See [LICENSE](LICENSE) for details.

<p align="center">
  <img src="docs/book/images/dashboard.png" alt="The BirdNet-Behavior dashboard" width="900">
</p>

---

## What is BirdNet-Behavior?

A ground-up Rust rewrite of [BirdNET-Pi](https://github.com/mcguirepr89/BirdNET-Pi). It runs on a Raspberry Pi, listens to your microphone or RTSP camera, identifies birds in real time using the BirdNET+ neural network, and serves a fast, beautiful web dashboard you open in any browser.

It ships as a **single static binary** — no Python, no pip, no virtualenv. Drop it on a Pi and run it.

| | BirdNET-Pi (Python) | BirdNet-Behavior (Rust) |
|---|---|---|
| Memory | 400–600 MB | ~20–50 MB |
| Cold start | 5–15 s | < 1 s |
| Dependencies | pip + venv + system libs | None — one binary |
| Upgrade | pip breakage, virtualenv rot | copy one file |
| Concurrency | GIL-constrained | Lock-free parallel audio |

> **It is a clean rewrite, not a fork.** See [Credits & Attribution](#credits--attribution).

---

## Screenshots

Every screen leads with a plain-English headline, then layers the dense numbers beneath — designed to serve a casual hobbyist and a PhD ornithologist at once. Light + dark themes, fully responsive.

<table>
  <tr>
    <td width="50%"><a href="https://tomtom215.github.io/BirdNet-Behavior/guide/today.html"><img src="docs/book/images/today.png" alt="Today's detections with a 24-hour timeline"></a><br><sub><b>Today</b> — searchable log + 24-hour DayStrip</sub></td>
    <td width="50%"><a href="https://tomtom215.github.io/BirdNet-Behavior/guide/analytics.html"><img src="docs/book/images/heatmap.png" alt="Activity heatmap and circadian polar"></a><br><sub><b>Analytics</b> — streamgraph, mosaic, circadian polar, ridgeline</sub></td>
  </tr>
  <tr>
    <td><a href="https://tomtom215.github.io/BirdNet-Behavior/guide/analytics.html"><img src="docs/book/images/correlation.png" alt="Co-occurrence chord diagram"></a><br><sub><b>Co-occurrence</b> — matrix + acoustic-network chord diagram</sub></td>
    <td><a href="https://tomtom215.github.io/BirdNet-Behavior/guide/species.html"><img src="docs/book/images/life-list.png" alt="Life list with accumulation curve"></a><br><sub><b>Life List</b> — birding journal with a growth curve</sub></td>
  </tr>
  <tr>
    <td><a href="https://tomtom215.github.io/BirdNet-Behavior/guide/reports.html"><img src="docs/book/images/year-in-review.png" alt="Year in Review"></a><br><sub><b>Year in Review</b> — editorial annual recap</sub></td>
    <td><a href="https://tomtom215.github.io/BirdNet-Behavior/guide/recordings.html"><img src="docs/book/images/gallery.png" alt="Species photo gallery"></a><br><sub><b>Gallery</b> — species photo grid</sub></td>
  </tr>
  <tr>
    <td><a href="https://tomtom215.github.io/BirdNet-Behavior/admin/audio.html"><img src="docs/book/images/admin-audio.png" alt="Audio settings"></a><br><sub><b>Audio setup</b> — USB + RTSP mic management</sub></td>
    <td><a href="https://tomtom215.github.io/BirdNet-Behavior/guide/dashboard.html"><img src="docs/book/images/dashboard-dark.png" alt="Dashboard in dark mode"></a><br><sub><b>Dark mode</b> — a cool observatory theme</sub></td>
  </tr>
</table>

➡️ **See every screen in the [Field Guide](https://tomtom215.github.io/BirdNet-Behavior/guide/dashboard.html).**

---

## Quick start

**On a Raspberry Pi (OS Trixie) or modern x86_64 Linux, install the native binary — it's the shortest path: one command, nothing else to set up.** It downloads and checksum-verifies the binary, pre-fetches the model, auto-detects your USB mic, asks for your location, and starts a hardened systemd service:

```bash
curl -fsSL https://raw.githubusercontent.com/tomtom215/BirdNet-Behavior/main/install.sh | sudo bash
```

> Run it exactly as shown — pipe into `sudo bash`, **not** `sudo bash <(curl …)` (the process-substitution form breaks under `sudo`). On a fresh 64-bit Raspberry Pi OS **Trixie** image the prebuilt binary runs as-is (glibc 2.39); the installer refuses early, with guidance, on anything older.

**Prefer Docker?** Use it if you're on **older Pi OS (Bookworm, glibc 2.36)** — the native binary won't run there — or you just want a container; it bundles its own runtime, so the host OS version doesn't matter. Raspberry Pi OS doesn't ship Docker, so install [Docker Engine](https://docs.docker.com/engine/install/) first (`curl -fsSL https://get.docker.com | sudo sh`), then run the one-liner below. It auto-detects your USB mic, asks for your location, writes a minimal `.env`, and starts the container:

```bash
bash <(curl -fsSL https://raw.githubusercontent.com/tomtom215/BirdNet-Behavior/main/quickstart.sh)
```

**macOS (Apple Silicon)** — the same installer detects macOS and sets up a per-user launchd LaunchAgent instead of systemd (**no `sudo`**):

```bash
curl -fsSL https://raw.githubusercontent.com/tomtom215/BirdNet-Behavior/main/install.sh | bash
```

> On macOS it installs the `aarch64-apple-darwin` build once a release publishes one; until then it offers to `brew install` the dependencies and prints the one-time source-build steps. A Homebrew formula is planned. (See [docs/MACOS.md](docs/MACOS.md).)

The BirdNET+ V3.0 model (~541 MB) downloads automatically from Zenodo on first run. When it's ready, open **<http://localhost:8502>** — or, from another device on your LAN, `http://<hostname>.local:8502` (or `http://<pi-ip>:8502` by IP; the installer prints both, and the dashboard binds to all interfaces by default). Viewing is open; only the `/admin` panel needs a login. The bare-metal installer auto-generates that admin password (user `birdnet`) and prints it once in its summary — save it. To restrict the dashboard to the machine itself, set `BIRDNET_LISTEN=127.0.0.1:8502`.

**Uninstall** (ships beside the binary, and as a release asset). It removes only the software by default — your database, recordings, and settings are kept unless you opt in:

```bash
sudo ./uninstall.sh              # remove the service + binary, KEEP all data
sudo ./uninstall.sh --dry-run    # preview exactly what would change
sudo ./uninstall.sh --purge      # remove everything, including data + model
```

📖 Full instructions: [Installation](https://tomtom215.github.io/BirdNet-Behavior/getting-started/installation.html) · [Docker guide](https://tomtom215.github.io/BirdNet-Behavior/getting-started/docker.html) · [Configuration](https://tomtom215.github.io/BirdNet-Behavior/getting-started/configuration.html)

---

## Supported hardware & OS

| Platform | Status | Notes |
|---|---|---|
| **Raspberry Pi 5 / 4B / 400** (64-bit) | ✅ Recommended | 64-bit Raspberry Pi OS **Trixie** |
| **x86_64 Linux** | ✅ Supported | glibc ≥ 2.39 (Debian 13 / Ubuntu 24.04+) |
| **Pi OS Bookworm** (glibc 2.36) | ⚠️ Docker only | Native binary needs glibc ≥ 2.39 — use Docker (bundles its own runtime) or upgrade to Trixie |
| **Pi 3 / Zero 2 W on 32-bit OS** (armv7) | ❌ | No prebuilt ONNX Runtime — reflash with 64-bit Pi OS |

**Runtime requirement: glibc ≥ 2.39** (Pi OS Trixie / Debian 13 / Ubuntu 24.04). The prebuilt binaries are built on Ubuntu 24.04 to match pyke's ONNX Runtime baseline; `install.sh` refuses to install on an older glibc and points you to Docker. Every prebuilt binary and the Docker image ship the DuckDB behavioral-analytics engine **built in and on by default** — the installer and Docker compose set the analytics database path automatically (for a manual binary run, pass `--analytics-db`; from source it is `cargo build --features analytics`).

---

## Features

**Everything BirdNET-Pi does** — real-time detection from a USB mic or RTSP stream, the BirdNET+ V3.0 model, a SQLite detection database, per-species pages, Apprise notifications (Telegram/Slack/Discord + 80 more), BirdWeather uploads, email alerts, CSV/JSON export, web-based admin, database backup/restore, and HTTP basic auth.

**New in BirdNet-Behavior:**

- **A redesigned UI** — 20+ pages, OKLCH light/dark themes, self-hosted fonts, bespoke SVG visualizations (streamgraph, circadian polar, co-occurrence chord diagram, migration ridgeline, DayStrip), fully responsive down to a phone.
- **Behavioral analytics** (built into every release, on by default; `--features analytics` for source builds) — activity sessions, resident vs. migrant classification, dawn-chorus validation, species co-occurrence, migration phenology.
- **IoT / Home Automation** — pure-Rust MQTT 3.1.1 publishing with Home Assistant auto-discovery.
- **Editorial reports** — Weekly Report and a celebratory Year in Review.
- **Share & follow** — a per-detection [detail page](https://tomtom215.github.io/BirdNet-Behavior/guide/sharing.html) with spectrogram + audio, signed public **share links** (`/r/<token>`, HMAC-SHA256, 30-day expiry), and **RSS/iCal feeds** for rare and daily detections. Print stylesheet for the reports.
- **Per-device display preferences** — theme, density, motion and contrast, applied before first paint (no flash on reload).
- **Operational polish** — rare-bird quarantine queue, audio quality pre-filtering, a built-in `--doctor` diagnostic, Prometheus metrics, kiosk mode, a live spectrogram, and a first-run onboarding wizard.

➡️ Tour them all in the [Field Guide](https://tomtom215.github.io/BirdNet-Behavior/guide/dashboard.html). New environment variables for these features — `BNB_SHARE_SECRET`, `BNB_BASE_URL`, `BNB_STATION_LAT`/`BNB_STATION_LON` — are documented in [`.env.example`](.env.example) and the [configuration reference](https://tomtom215.github.io/BirdNet-Behavior/reference/configuration-reference.html).

---

## Migrating from BirdNET-Pi

Safe, non-destructive import — the source database is opened read-only and never modified. Stop BirdNET-Pi, open `/admin/migrate`, point it at your `BirdDB.txt`, review the preview, and import. Duplicate rows are skipped, so re-running is safe.

📖 [Migration guide](https://tomtom215.github.io/BirdNet-Behavior/guides/migration.html)

---

## Building from source

**Prerequisites:** [Rust 1.95+](https://rustup.rs) and `git`.

```bash
git clone https://github.com/tomtom215/BirdNet-Behavior.git
cd BirdNet-Behavior

cargo build --release                              # optimized build
cargo build --release --features analytics         # + DuckDB analytics
cross build --release --target aarch64-unknown-linux-gnu   # cross-compile for a Pi

cargo test --workspace                             # run tests
cargo clippy --workspace --all-targets -- -D warnings   # lint (pedantic + nursery)
```

---

## Troubleshooting

First step for any problem — run the built-in diagnostic, which prints a one-screen report (CPU, config, audio reachability, model, database, disk, network) with a concrete fix for each issue:

```bash
sudo -u birdnet birdnet-behavior --doctor            # bare metal
docker compose exec birdnet birdnet-behavior --doctor   # Docker
```

📖 [Troubleshooting guide](https://tomtom215.github.io/BirdNet-Behavior/guides/troubleshooting.html) · deeper recipes in [`TROUBLESHOOTING.md`](TROUBLESHOOTING.md).

---

## Architecture

A single binary built from eight Rust workspace crates — `birdnet-core` (audio + ML), `birdnet-db` (SQLite + DuckDB), `birdnet-web` (axum + HTMX), `birdnet-integrations`, `birdnet-behavioral`, `birdnet-timeseries`, `birdnet-migrate`, and `birdnet-scheduler`.

📖 [Architecture overview](https://tomtom215.github.io/BirdNet-Behavior/reference/architecture.html) · full design docs in [`docs/architecture/`](docs/architecture/).

---

## Documentation

The complete, navigable documentation lives at **[tomtom215.github.io/BirdNet-Behavior](https://tomtom215.github.io/BirdNet-Behavior/)** — installation, a screen-by-screen field guide, configuration, administration, migration, an FAQ, and troubleshooting. It is built with [mdBook](https://rust-lang.github.io/mdBook/) from the Markdown in [`docs/book/`](docs/book/) and published automatically on every push to `main`.

---

## Credits & Attribution

- **[BirdNET](https://github.com/kahst/BirdNET-Analyzer)** — ML model by the K. Lisa Yang Center for Conservation Bioacoustics, Cornell Lab of Ornithology
- **[BirdNET-Pi](https://github.com/mcguirepr89/BirdNET-Pi)** — original Pi implementation by [Patrick McGuire](https://github.com/mcguirepr89)
- **[BirdNET-Pi fork](https://github.com/Nachtzuster/BirdNET-Pi)** — maintained fork by [Nachtzuster](https://github.com/Nachtzuster)
- **[duckdb-behavioral](https://github.com/tomtom215/duckdb-behavioral)** — behavioral analytics by [tomtom215](https://github.com/tomtom215)

---

## License

Licensed under [CC BY-NC-SA 4.0](https://creativecommons.org/licenses/by-nc-sa/4.0/), matching the upstream BirdNET and BirdNET-Pi projects. See [LICENSE](LICENSE) and [LICENSE-UPSTREAM](LICENSE-UPSTREAM) for full terms.

---

## Related Projects

| Repository | Description |
|---|---|
| [duckdb-behavioral](https://github.com/tomtom215/duckdb-behavioral) | ClickHouse-inspired behavioral analytics for DuckDB |
| [quack-rs](https://github.com/tomtom215/quack-rs) | SDK for building DuckDB extensions in Rust |
| [mallardmetrics](https://github.com/tomtom215/mallardmetrics) | Single-binary web analytics (axum + DuckDB) |
| [LyreBirdAudio](https://github.com/tomtom215/LyreBirdAudio) | RTSP audio streaming |
