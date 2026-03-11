<h1 align="center">
  BirdNet-Behavior
</h1>
<p align="center">
Real-time acoustic bird classification with behavioral analytics, written in Rust
</p>

<p align="center">
  <a href="https://creativecommons.org/licenses/by-nc-sa/4.0/"><img src="https://img.shields.io/badge/License-CC%20BY--NC--SA%204.0-lightgrey.svg"></a>
</p>

<h2 align="center"><a href="LICENSE">Review the license!</a></h2>
<h3 align="center">You may not use BirdNet-Behavior to develop a commercial product!</h3>

## About

BirdNet-Behavior is a ground-up Rust rewrite of [BirdNET-Pi](https://github.com/mcguirepr89/BirdNET-Pi), designed for resource-constrained and solar-powered bird monitoring stations. It ships as a **single binary** with no Python, no pip, no virtualenv -- just `scp` it to your Pi and run.

### What's New

**Behavioral Analytics** powered by [duckdb-behavioral](https://github.com/tomtom215/duckdb-behavioral):
- Activity sessionization -- understand when birds are actively vocalizing vs. passing through
- Species retention analysis -- distinguish residents from migrants and rarities
- Dawn chorus funnel analysis -- validate species ordering patterns at dawn
- Sequence pattern matching -- discover if species A predicts species B
- "What comes next?" predictions -- real-time species prediction for the web UI

**Dual-Database Architecture:**
- **SQLite** for real-time OLTP (detection writes, live queries)
- **DuckDB** for columnar OLAP analytics (trends, aggregations, behavioral patterns)

**Rust Advantages over Python:**
- ~20-50 MB RSS vs 400-600 MB (Python + librosa + TFLite)
- <1 second cold start vs 5-15 seconds
- Single binary deployment -- no dependency rot, no broken pip installs
- Deterministic latency -- no GC pauses during real-time audio analysis
- Safe concurrency -- parallel audio processing without GIL contention

## Architecture

```
birdnet-behavior (single binary)
├── Core Engine (birdnet-core)
│   ├── Audio Capture (mic/RTSP)
│   ├── Audio Processing (symphonia + rubato + mel_spec)
│   ├── ML Inference (ONNX Runtime via ort)
│   └── Detection Pipeline (watch → analyze → report)
│
├── Data Layer (birdnet-db)
│   ├── SQLite (operational: detections, real-time queries)
│   ├── DuckDB (analytics: trends, aggregations)
│   └── Resilience (WAL, backup, integrity, recovery)
│
├── Web Server (birdnet-web)
│   ├── REST API (/api/v2/*)
│   ├── WebSocket (/ws/detections)
│   └── HTMX UI (embedded via rust-embed)
│
├── Behavioral Analytics (birdnet-behavioral)
│   ├── Activity sessions, retention, funnels
│   └── Sequence patterns, predictions
│
└── Integrations (birdnet-integrations)
    ├── BirdWeather
    ├── Apprise notifications
    └── Flickr/Wikipedia image caching
```

## Requirements

- Raspberry Pi 5, 4B, or 400 (64-bit required) -- or any x86_64 Linux
- A USB microphone or sound card
- That's it. No Python. No pip. No system dependencies.

## Installation

```bash
# Download the latest release for your platform
curl -L https://github.com/tomtom215/BirdNet-Behavior/releases/latest/download/birdnet-behavior-aarch64 \
  -o /usr/local/bin/birdnet-behavior
chmod +x /usr/local/bin/birdnet-behavior

# Run it
birdnet-behavior
```

## Building from Source

```bash
# Clone
git clone https://github.com/tomtom215/BirdNet-Behavior.git
cd BirdNet-Behavior

# Build (debug)
cargo build

# Build (release, optimized)
cargo build --release

# Cross-compile for Raspberry Pi
cross build --release --target aarch64-unknown-linux-gnu
```

### MSRV

Rust 1.85+ (edition 2024)

## Credits & Attribution

BirdNet-Behavior is built on the shoulders of these excellent projects:

- **[BirdNET](https://github.com/kahst/BirdNET-Analyzer)** by [@kahst](https://github.com/kahst) -- the ML framework and models for bird sound classification (K. Lisa Yang Center for Conservation Bioacoustics, Cornell Lab of Ornithology)
- **[BirdNET-Pi](https://github.com/mcguirepr89/BirdNET-Pi)** by [Patrick McGuire](https://github.com/mcguirepr89) -- the original Raspberry Pi implementation
- **[BirdNET-Pi fork](https://github.com/Nachtzuster/BirdNET-Pi)** by [Nachtzuster](https://github.com/Nachtzuster) -- maintained fork with Bookworm support, backup/restore, and many improvements
- **[duckdb-behavioral](https://github.com/tomtom215/duckdb-behavioral)** by [tomtom215](https://github.com/tomtom215) -- ClickHouse-inspired behavioral analytics for DuckDB

This project is **not a fork** -- it is a clean Rust rewrite. The architecture, design decisions, and feature analysis were informed by [tomtom215's BirdNET-Pi fork](https://github.com/tomtom215/BirdNET-Pi).

## License

BirdNet-Behavior is licensed under the [Creative Commons Attribution-NonCommercial-ShareAlike 4.0 International License](https://creativecommons.org/licenses/by-nc-sa/4.0/), matching the upstream BirdNET and BirdNET-Pi projects.

See [LICENSE](LICENSE) and [LICENSE-UPSTREAM](LICENSE-UPSTREAM) for full details.

## Related Projects

| Repository | Description |
|---|---|
| [duckdb-behavioral](https://github.com/tomtom215/duckdb-behavioral) | ClickHouse-inspired behavioral analytics for DuckDB |
| [quack-rs](https://github.com/tomtom215/quack-rs) | SDK for building DuckDB extensions in Rust |
| [mallardmetrics](https://github.com/tomtom215/mallardmetrics) | Single-binary web analytics (axum + DuckDB) |
| [LyreBirdAudio](https://github.com/tomtom215/LyreBirdAudio) | RTSP audio streaming |
