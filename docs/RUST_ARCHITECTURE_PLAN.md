# BirdNET-Pi Rust Architecture Plan

> A phased plan to rewrite BirdNET-Pi's core in Rust for reliability, efficiency,
> and sustainability on resource-constrained field deployments.
>
> **Author:** tomtom215 | **Updated:** 2026-03-13

## Overview

BirdNet-Behavior is a real-time acoustic bird classification system targeting
Raspberry Pi (5, 4B, 400) and x86_64 Linux. It replaces the Python BirdNET-Pi
with a single Rust binary that integrates
[duckdb-behavioral](https://github.com/tomtom215/duckdb-behavioral) for bird
activity analytics.

**Design principles:**
- Single binary deployment -- no pip, no venv, no apt dependencies
- Minimal dependencies -- pure Rust wherever possible
- Zero C dependencies in the audio pipeline
- `unsafe` denied workspace-wide
- 20-50 MB RSS on a Raspberry Pi 4

## Architecture Documents

Detailed documentation is organized into focused modules:

| # | Document | Description |
|---|----------|-------------|
| 01 | [Motivation](architecture/01-motivation.md) | Why Rust, why not Python/Go, design philosophy |
| 02 | [Architecture](architecture/02-architecture.md) | Single binary design, workspace layout, crate responsibilities |
| 03 | [Coding Standards](architecture/03-coding-standards.md) | Linting, error handling, async conventions, testing |
| 04 | [Dependencies](architecture/04-dependencies.md) | Minimal deps policy, pure Rust focus, supply chain security |
| 05 | [Audio Pipeline](architecture/05-audio-pipeline.md) | Decode, resample, mel spectrogram (pure Rust) |
| 06 | [ML Inference](architecture/06-ml-inference.md) | ONNX Runtime, tract fallback, model variants |
| 07 | [Database](architecture/07-database.md) | SQLite OLTP + DuckDB OLAP dual-database design |
| 08 | [Behavioral Analytics](architecture/08-behavioral-analytics.md) | duckdb-behavioral integration, ecological queries |
| 09 | [Web Server](architecture/09-web-server.md) | axum REST API, WebSocket, HTMX |
| 10 | [Deployment](architecture/10-deployment.md) | Cross-compilation, CI/CD, systemd, resource targets |
| 11 | [Migration](architecture/11-migration.md) | Python coexistence, backwards compatibility, rollback |
| 12 | [Risks](architecture/12-risks.md) | Risk matrix, critical path, mitigations |
| 13 | [Implementation Status](architecture/13-implementation-status.md) | Current progress, what's built vs. planned |

## Quick Status

| Phase | Description | Status |
|-------|------------|--------|
| 0 | Scaffolding | **Complete** |
| 1 | Data Layer (SQLite) | **Complete** |
| 2 | Audio Pipeline | Partial (mel spectrogram pending) |
| 3 | ML Inference | Not started |
| 4 | Detection Daemon | Not started |
| 5 | Web Server | Partial (core API done) |
| 6 | Integrations | Partial (BirdWeather done) |
| 7 | Audio Capture | Not started |
| 8 | Single Binary | Partial (main.rs + CLI done) |

See [Implementation Status](architecture/13-implementation-status.md) for details.

## Workspace Structure

```
BirdNet-Behavior/
├── Cargo.toml              # Workspace root
├── src/main.rs             # Binary entry point
├── crates/
│   ├── birdnet-core/       # Audio, config, detection types (sync, pure Rust)
│   ├── birdnet-db/         # SQLite operations + resilience (sync)
│   ├── birdnet-web/        # axum REST API + WebSocket (async)
│   ├── birdnet-integrations/ # BirdWeather, notifications (async)
│   └── birdnet-behavioral/ # DuckDB analytics types + SQL builders
├── docs/
│   ├── RUST_ARCHITECTURE_PLAN.md  # This file (index)
│   └── architecture/              # Detailed architecture documents
└── tests/                  # Integration tests
```

## Key Dependencies

All pure Rust except where noted:

| Purpose | Crate | Pure Rust |
|---------|-------|-----------|
| Audio decode | `symphonia` | Yes |
| Resampling | `rubato` | Yes |
| File watching | `notify` | Yes |
| Web framework | `axum` | Yes |
| HTTP client | `reqwest` | Yes |
| SQLite | `rusqlite` (bundled) | No (bundles C source) |
| ML inference | `ort` / `tract` | No / Yes |

## tomtom215 Rust Ecosystem

| Repository | Purpose | Relevance |
|---|---|---|
| [duckdb-behavioral](https://github.com/tomtom215/duckdb-behavioral) | ClickHouse-inspired analytics | Bird activity analytics |
| [quack-rs](https://github.com/tomtom215/quack-rs) | SDK for DuckDB Rust extensions | Extension infrastructure |
| [mallardmetrics](https://github.com/tomtom215/mallardmetrics) | Single-binary web analytics | Architecture template |
| [duckdb-rs](https://github.com/tomtom215/duckdb-rs) | DuckDB Rust bindings | Direct dependency |
| [LyreBirdAudio](https://github.com/tomtom215/LyreBirdAudio) | RTSP audio streaming | Audio capture patterns |

---

*This document serves as the index for the BirdNet-Behavior architecture plan.
See the [architecture/](architecture/) directory for detailed documentation.*
