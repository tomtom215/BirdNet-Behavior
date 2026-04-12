# BirdNet-Behavior Architecture

> Design and implementation notes for a Rust rewrite of BirdNET-Pi
> targeting resource-constrained field deployments.

## Overview

BirdNet-Behavior is a real-time acoustic bird classification system for
Raspberry Pi (5, 4B, 400) and x86_64 Linux. It replaces the Python
BirdNET-Pi stack with a single Rust binary and integrates
[duckdb-behavioral](https://github.com/tomtom215/duckdb-behavioral) for
bird activity analytics.

**Design principles:**

- Single binary deployment — no pip, no virtualenv, no apt dependencies
- Pure Rust wherever practical; minimal external surface
- Zero C dependencies in the audio pipeline
- `unsafe` denied workspace-wide
- 20–50 MB RSS on a Raspberry Pi 4

## Architecture Documents

Detailed documentation is split into focused modules:

| # | Document | Description |
|---|----------|-------------|
| 01 | [Motivation](architecture/01-motivation.md) | Why Rust, why not Python or Go, design philosophy |
| 02 | [Architecture](architecture/02-architecture.md) | Single binary design, workspace layout, crate responsibilities |
| 03 | [Coding Standards](architecture/03-coding-standards.md) | Linting, error handling, async conventions, testing |
| 04 | [Dependencies](architecture/04-dependencies.md) | Minimal deps policy, pure Rust focus, supply chain |
| 05 | [Audio Pipeline](architecture/05-audio-pipeline.md) | Decode, resample, mel spectrogram (pure Rust) |
| 06 | [ML Inference](architecture/06-ml-inference.md) | ONNX Runtime, model variants, inference pipeline |
| 07 | [Database](architecture/07-database.md) | SQLite OLTP + DuckDB OLAP dual-database design |
| 08 | [Behavioral Analytics](architecture/08-behavioral-analytics.md) | duckdb-behavioral integration, ecological queries |
| 09 | [Web Server](architecture/09-web-server.md) | axum REST API, WebSocket, HTMX pages |
| 10 | [Deployment](architecture/10-deployment.md) | Cross-compilation, CI/CD, systemd, resource targets |
| 11 | [Migration](architecture/11-migration.md) | BirdNET-Pi import, schema detection, rollback |
| 12 | [Risks](architecture/12-risks.md) | Risk matrix, critical path, mitigations |
| 13 | [Implementation Status](architecture/13-implementation-status.md) | Current status per crate and test coverage |

## Workspace Structure

```
BirdNet-Behavior/
├── Cargo.toml              # Workspace root
├── src/                    # Binary entry point and application glue
│   ├── main.rs
│   ├── cli.rs
│   ├── daemon.rs
│   ├── capture.rs
│   ├── integrations.rs
│   ├── helpers.rs
│   └── weekly_report.rs
├── crates/
│   ├── birdnet-core/         # Audio, detection pipeline, ML inference
│   ├── birdnet-db/           # SQLite + resilience + migrations
│   ├── birdnet-web/          # axum REST API, WebSocket, HTMX
│   ├── birdnet-integrations/ # BirdWeather, Apprise, email, Wikipedia, MQTT
│   ├── birdnet-behavioral/   # DuckDB behavioral analytics (feature-gated)
│   ├── birdnet-timeseries/   # Activity, diversity, trend, peak, gap, sessions
│   ├── birdnet-migrate/      # BirdNET-Pi schema detection and import
│   └── birdnet-scheduler/    # Solar calculations and recording windows
├── docs/                   # Architecture documentation and screenshots
└── tests/                  # Integration tests
```

## Key Dependencies

All pure Rust except where noted:

| Purpose | Crate | Pure Rust |
|---------|-------|-----------|
| Audio decode | `symphonia` | Yes |
| Resampling | `rubato` | Yes |
| FFT | `realfft` | Yes |
| File watching | `notify` | Yes |
| Web framework | `axum` | Yes |
| HTTP client | `reqwest` (rustls) | Yes |
| Email (SMTP) | `lettre` (rustls) | Yes |
| SQLite | `rusqlite` (bundled) | No (bundles C source) |
| DuckDB | `duckdb` (bundled, optional) | No (bundles C++ source) |
| ML inference | `ort` (ONNX Runtime) | No |

---

*This document serves as the index for the BirdNet-Behavior architecture.
See the [architecture/](architecture/) directory for detailed documentation.*
