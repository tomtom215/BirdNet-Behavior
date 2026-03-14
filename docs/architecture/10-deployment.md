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
| `armv7-unknown-linux-gnueabihf` | Pi Zero 2W (32-bit compat) | Tertiary |

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

```yaml
name: Release
on:
  push:
    tags: ['v*']

jobs:
  build:
    strategy:
      matrix:
        include:
          - target: x86_64-unknown-linux-gnu
            os: ubuntu-latest
          - target: aarch64-unknown-linux-gnu
            os: ubuntu-latest
          - target: armv7-unknown-linux-gnueabihf
            os: ubuntu-latest

    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@<commit-sha>
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}
      - uses: cross-rs/cross-action@<commit-sha>
        with:
          command: build
          args: --release --target ${{ matrix.target }}
      - uses: actions/upload-artifact@<commit-sha>
        with:
          name: birdnet-behavior-${{ matrix.target }}
          path: target/${{ matrix.target }}/release/birdnet-behavior
```

Action versions pinned by **commit SHA** (not tags) for reproducibility.

## Quality Pipeline

Following duckdb-behavioral's proven 6-workflow pattern:

1. **Quality**: `fmt` → `clippy` → `check` → `doc` (fail fast)
2. **Testing**: `cargo nextest run` on Ubuntu + macOS, MSRV verification
3. **Security**: `cargo-deny` supply chain audit, CodeQL static analysis
4. **Compatibility**: SemVer check via `cargo-semver-checks`
5. **Coverage**: `cargo-tarpaulin` → Codecov
6. **Release**: Multi-platform builds with provenance attestation

Concurrency cancellation for redundant PR runs.

## Deployment

```bash
# On the Pi:
curl -L https://github.com/tomtom215/BirdNet-Behavior/releases/latest/download/birdnet-behavior-aarch64 \
  -o /usr/local/bin/birdnet-behavior
chmod +x /usr/local/bin/birdnet-behavior
systemctl restart birdnet-behavior

# That's it. No pip. No venv. No apt dependencies.
```

### Migrate from BirdNET-Pi (Zero-Downtime)

```bash
# 1. Stop BirdNET-Pi (optional — migration is non-destructive read-only)
systemctl stop birdnet_analysis birdnet_web

# 2. Start BirdNet-Behavior
systemctl start birdnet-behavior

# 3. Open the web UI at http://pi.local:7070
# 4. Navigate to Admin → Migration → Upload BirdNET-Pi database
# 5. Validate the import report, then confirm import
# 6. Your existing installation is untouched
```

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
    --port 7070
Restart=on-failure
RestartSec=5
WatchdogSec=60
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

### Key CLI Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--config` | `/etc/birdnet/birdnet.conf` | birdnet.conf path |
| `--db` | `./birds.db` | SQLite database path |
| `--port` | `7070` | HTTP server port |
| `--backup-dir` | `./backups/` | Backup file directory |
| `--recordings-dir` | `./StreamData/` | WAV recordings directory |
| `--log-level` | `info` | Log verbosity (trace/debug/info/warn/error) |

---

*Last updated: 2026-03-14*

[← Web Server](09-web-server.md) | [Back to Index](../RUST_ARCHITECTURE_PLAN.md) | [Next: Migration →](11-migration.md)
