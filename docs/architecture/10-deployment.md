# Cross-Compilation & Deployment

> Build on CI, deploy a single binary to any target.

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

### systemd Service

```ini
[Unit]
Description=BirdNet-Behavior Detection System
After=network.target sound.target

[Service]
Type=simple
ExecStart=/usr/local/bin/birdnet-behavior --config /etc/birdnet/birdnet.conf
Restart=on-failure
RestartSec=5
WatchdogSec=60
MemoryMax=256M
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
```

## Resource Expectations

| Metric | Python (current) | Rust (target) |
|--------|-----------------|---------------|
| Binary/install size | ~500 MB (venv + deps) | ~15-30 MB (single binary) |
| RSS memory (idle) | ~200 MB | ~10-20 MB |
| RSS memory (analyzing) | ~400-600 MB | ~30-50 MB |
| Cold start time | 5-15 seconds | <1 second |
| Inference latency (3s clip) | ~1-2 seconds | ~0.5-1 second |
| Disk write durability | WAL (just added) | WAL + fsync control |

---

[← Web Server](09-web-server.md) | [Back to Index](../RUST_ARCHITECTURE_PLAN.md) | [Next: Migration →](11-migration.md)
