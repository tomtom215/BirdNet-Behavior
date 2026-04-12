# Cross-Compilation & Deployment

> Build on CI, deploy a single binary to any target.

## Table of Contents

- [Target Platforms](#target-platforms)
- [Cross-Compile Difficulty by Dependency](#cross-compile-difficulty-by-dependency)
- [Build Strategy](#build-strategy)
- [CI/CD Pipeline](#cicd-pipeline-github-actions)
- [Deployment](#deployment)
- [systemd Service](#systemd-service)
- [Resource Expectations](#resource-expectations)
- [Configuration](#configuration)

---

## Target Platforms

| Target | Hardware | Priority |
|--------|----------|----------|
| `aarch64-unknown-linux-gnu` | Raspberry Pi 5, Pi 4B, Pi 400 | Primary |
| `x86_64-unknown-linux-gnu` | Desktop Linux, servers | Secondary |

`armv7-unknown-linux-gnueabihf` (Pi 3 / Pi Zero 2W in 32-bit mode) is
not shipped as a release binary: the `ort` crate does not provide
prebuilt ONNX Runtime archives for armv7, and building ORT from source
adds ~30 minutes of CI time per release.  32-bit ARM users should
install the 64-bit Raspberry Pi OS and run the aarch64 binary, or
build from source.

## Cross-Compile Difficulty by Dependency

| Library | Difficulty | Notes |
|---------|-----------|-------|
| symphonia, rubato | **None** | Pure Rust |
| axum, notify, serde, clap | **None** | Pure Rust |
| lettre (rustls) | **None** | Pure Rust TLS |
| rusqlite (bundled) | **None** | Bundles SQLite C source, cc compiles it |
| `ort` (ONNX Runtime) | **Medium** | Pre-built aarch64 binaries auto-downloaded |
| `duckdb` | **Medium-High** | Needs C++ cross-toolchain; custom Docker image |

## Build Strategy

**Approach:** `cross-rs` with custom Docker images for targets needing C++ toolchains.

```bash
# Install cross
cargo install cross

# Build for Pi
cross build --release --target aarch64-unknown-linux-gnu

# Or with cargo-zigbuild (simpler, uses Zig's universal linker)
cargo install cargo-zigbuild
cargo zigbuild --release --target aarch64-unknown-linux-gnu
```

For pure Rust dependencies only (no DuckDB), standard cross-compilation works:
```bash
rustup target add aarch64-unknown-linux-gnu
cargo build --release --target aarch64-unknown-linux-gnu
```

## CI/CD Pipeline (GitHub Actions)

Release artifacts are produced by `.github/workflows/release.yml`. The
workflow cross-compiles the `aarch64` and `x86_64` GNU Linux targets on
Ubuntu 24.04 using the native GCC 13 cross toolchain (glibc 2.39
baseline) — this matches the libstdc++ and glibc baselines that pyke's
prebuilt ONNX Runtime archives were built against, so release binaries
require **glibc 2.39 or newer** (Raspberry Pi OS Trixie, Debian 13,
Ubuntu 24.04, or newer). See the workflow header comments for the full
diagnosis.

## Quality Pipeline

The CI workflow (`.github/workflows/ci.yml`) runs six jobs on every push
and pull request:

1. **fmt** — `cargo fmt --check --all`
2. **clippy** — `cargo clippy --workspace --all-targets -- -D warnings`
3. **test** — unit, integration, and doc tests
4. **doc** — `cargo doc --workspace --no-deps` with broken-link denial
5. **build** — debug build with and without the `analytics` feature
6. **msrv** — `cargo check` against the declared MSRV

A separate workflow (`.github/workflows/docker.yml`) assembles a
multi-architecture container image on native `ubuntu-24.04` (amd64) and
`ubuntu-24.04-arm` (arm64) runners, then merges per-platform digests into
a single manifest list.

## Deployment

The recommended path on a Raspberry Pi or bare-metal Linux host is the
installer script bundled with each release:

```bash
curl -fsSL https://raw.githubusercontent.com/tomtom215/BirdNet-Behavior/main/install.sh | sudo bash
```

The installer detects the host architecture, downloads the matching
pre-built binary from GitHub Releases, downloads the BirdNET+ V3.0 model
from Zenodo, writes the systemd unit, and starts the service.

For manual installation:

```bash
# Download the release binary for the target architecture
curl -L https://github.com/tomtom215/BirdNet-Behavior/releases/latest/download/birdnet-behavior-aarch64-unknown-linux-gnu \
  -o /usr/local/bin/birdnet-behavior
chmod +x /usr/local/bin/birdnet-behavior

# Enable and start the systemd service
sudo systemctl enable --now birdnet-behavior
```

### Migrate from BirdNET-Pi

Migration is non-destructive: the source BirdNET-Pi database is opened
read-only and never modified.

1. Start BirdNet-Behavior (it can run alongside BirdNET-Pi)
2. Open the web UI at `http://<host>:8502/admin/migrate`
3. Upload `BirdDB.txt` or point to the BirdNET-Pi SQLite database
4. Review the preview report (top species, date range, data quality)
5. Click Import — transactional, fails cleanly on any error
6. Verify the per-species count comparison

## systemd Service

```ini
[Unit]
Description=BirdNet-Behavior Detection System
After=network.target sound.target
StartLimitIntervalSec=60
StartLimitBurst=3

[Service]
Type=simple
ExecStart=/usr/local/bin/birdnet-behavior \
    --config /etc/birdnet/birdnet.conf \
    --db /var/lib/birdnet/birds.db \
    --listen 0.0.0.0:8502
Restart=on-failure
RestartSec=5
MemoryMax=256M
StandardOutput=journal
StandardError=journal
SyslogIdentifier=birdnet-behavior

# Hardening (optional, Pi-compatible)
NoNewPrivileges=true
PrivateTmp=true

[Install]
WantedBy=multi-user.target
```

### Install the service

```bash
sudo cp birdnet-behavior.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now birdnet-behavior
```

## Resource Expectations

| Metric | Python BirdNET-Pi | Rust BirdNet-Behavior |
|--------|------------------|----------------------|
| Binary / install size | ~500 MB (venv + deps) | ~15–30 MB (single binary) |
| RSS memory (idle) | ~200 MB | ~10–20 MB |
| RSS memory (analyzing) | ~400–600 MB | ~30–50 MB |
| Cold start time | 5–15 seconds | <1 second |
| Inference latency (3s clip) | ~1–2 seconds | ~0.5–1 second |
| Disk write durability | WAL | WAL + fsync control |
| Number of systemd services | 6+ | 1 |

## Configuration

Configuration priority order (highest to lowest):

1. CLI flags (`--port`, `--db`, `--config`)
2. Environment variables (`BIRDNET_PORT`, `BIRDNET_DB`)
3. Settings stored in the SQLite `settings` table (editable via web UI)
4. `birdnet.conf` INI file (for BirdNET-Pi compatibility)
5. Built-in defaults

### Selected CLI Flags

| Flag | Description |
|------|-------------|
| `--config` | Path to `birdnet.conf` (INI) |
| `--db` | SQLite database path |
| `--listen` | HTTP listen address (default `0.0.0.0:8502`) |
| `--model` / `--labels` | BirdNET model and labels file paths |
| `--watch-dir` | Directory the detection daemon watches for new recordings |
| `--alsa-device` / `--rtsp-url` / `--rtsp-urls` | Audio source selection |
| `--analytics-db` | DuckDB database path (enables behavioral analytics endpoints) |
| `--apprise-url`, `--birdweather-token`, `--mqtt-host` | Integrations |

See `birdnet-behavior --help` for the full list of flags and their
environment variable equivalents.

---

[← Web Server](09-web-server.md) | [Back to Index](../RUST_ARCHITECTURE_PLAN.md) | [Next: Migration →](11-migration.md)
