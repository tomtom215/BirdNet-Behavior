# BirdNet-Behavior Repository Reference

**A Rust rewrite of BirdNET-Pi with DuckDB behavioral analytics.**

BirdNet-Behavior is a real-time acoustic bird classification system targeting
Raspberry Pi (5, 4B, 400) and x86_64 Linux, built as a single Rust binary.
It integrates [duckdb-behavioral](https://github.com/tomtom215/duckdb-behavioral)
for bird activity analytics (sessionization, retention, funnel analysis, sequence matching).

## Lineage & Attribution

This project is derived from BirdNET-Pi (CC BY-NC-SA 4.0):
- **BirdNET**: K. Lisa Yang Center for Conservation Bioacoustics, Cornell University
- **BirdNET-Pi**: Patrick McGuire (mcguirepr89)
- **BirdNET-Pi fork**: Nachtzuster
- **BirdNET-Pi fork**: tomtom215

See `LICENSE` and `LICENSE-UPSTREAM` for full attribution and license terms.

## Architecture

Single Rust binary with 8 workspace crates:

| Crate | Purpose |
|-------|---------|
| `birdnet-core` | Audio capture, decode, resample, mel spectrogram, ML inference, detection pipeline, tmpfs, live spectrogram |
| `birdnet-db` | SQLite (OLTP) + DuckDB (OLAP), resilience, migrations |
| `birdnet-web` | axum web server, REST API, WebSocket, HTMX templates, audio player, admin |
| `birdnet-integrations` | BirdWeather, Apprise, Wikipedia images, email, auto-update |
| `birdnet-behavioral` | DuckDB behavioral analytics for bird activity patterns |
| `birdnet-timeseries` | Time-series analytics (activity, diversity, trend, peak, gap, sessions) |
| `birdnet-migrate` | BirdNET-Pi migration: schema detection, validation, import |
| `birdnet-scheduler` | Solar calculations, recording window scheduling |

See `docs/RUST_ARCHITECTURE_PLAN.md` for the full phased implementation plan.

## Quick Reference

### Build & Test

```bash
# Build (debug)
cargo build

# Build (release, optimized for Pi deployment)
cargo build --release

# Run tests
cargo test

# Run with clippy lints
cargo clippy --workspace --all-targets

# Format check
cargo fmt --check --all
```

### Cross-compilation (for Raspberry Pi)

```bash
# Install target
rustup target add aarch64-unknown-linux-gnu

# Build with cross
cross build --release --target aarch64-unknown-linux-gnu
```

### Coding Conventions

- **No `anyhow`/`thiserror` in library crates** - hand-rolled error types
- **No async in library crates** - birdnet-core, birdnet-db are synchronous
- **Tokio only in application code** (birdnet-web, main binary)
- **Blocking ops via `tokio::task::spawn_blocking`** for DB, file I/O, inference
- **`unsafe` is denied** workspace-wide
- **Clippy pedantic + nursery** enabled

### Key Dependencies

| Purpose | Crate |
|---------|-------|
| Async runtime | `tokio` |
| Web framework | `axum` |
| SQLite | `rusqlite` (bundled) |
| Audio decode | `symphonia` |
| Resampling | `rubato` |
| ML inference | `ort` (ONNX Runtime) |
| File watching | `notify` |
| Logging | `tracing` |

### MSRV

Rust 1.85 (edition 2024)

---

*Modular reference for BirdNet-Behavior. Last updated: 2026-03-20*
