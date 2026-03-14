# Target Architecture

> Single binary design with 7 workspace crates.

## Table of Contents

- [Single Binary Design](#single-binary-design)
- [Workspace Layout](#workspace-layout)
- [Crate Responsibilities](#crate-responsibilities)
- [Inter-Crate Dependencies](#inter-crate-dependencies)

---

## Single Binary Design

Inspired by tomtom215's `mallardmetrics` pattern: a single Rust binary that embeds
all functionality, deployed as one file.

```
birdnet-behavior (single binary)
├── Core Engine
│   ├── Audio Capture (replaces birdnet_recording.sh)
│   ├── ML Inference (ONNX via ort / pure Rust via tract)
│   ├── Detection Pipeline (notify → analyze → report)
│   └── Audio Processing (decode, resample, mel spectrogram)
│
├── Data Layer
│   ├── SQLite (operational: detections, settings, real-time queries)
│   ├── DuckDB (analytics: trends, aggregations, behavioral)
│   └── Resilience (WAL, backup, integrity, recovery)
│
├── Web Server (axum)
│   ├── REST API (/api/v2/*)
│   ├── Server-Sent Events (/api/v2/detections/stream, /api/v2/logs/stream)
│   ├── HTMX pages (dashboard, species, heatmap, analytics, admin)
│   └── Admin panel (settings, system, backup, logs)
│
├── Integrations
│   ├── BirdWeather (reqwest + retry queue)
│   ├── Email alerts (lettre, SMTP, per-species cooldown)
│   ├── Apprise notifications
│   ├── Image caching (Flickr/Wikipedia)
│   └── RTSP/Icecast
│
├── Time Series
│   └── Trend analysis, moving averages, seasonal patterns
│
└── Migration
    └── BirdNET-Pi SQLite import (zero-downtime, non-destructive)
```

## Workspace Layout

```
BirdNet-Behavior/
├── Cargo.toml                    # Workspace root
├── crates/
│   ├── birdnet-core/             # Detection pipeline, audio, ML
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── config.rs         # birdnet.conf parser (INI-style)
│   │       ├── audio/
│   │       │   ├── mod.rs
│   │       │   ├── capture.rs    # Mic/RTSP recording subprocess
│   │       │   ├── decode.rs     # WAV/FLAC/MP3 via symphonia ✅
│   │       │   ├── resample.rs   # Via rubato ✅
│   │       │   └── spectrogram.rs # Mel spectrogram (pure Rust) ⚠️
│   │       └── detection/
│   │           ├── mod.rs
│   │           ├── pipeline.rs   # Watch → Analyze → Report ✅
│   │           └── types.rs      # Detection, RecordingFile types ✅
│   │
│   ├── birdnet-db/               # Database layer
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── duckdb/           # OLAP analytics queries
│   │       │   ├── mod.rs
│   │       │   ├── connection.rs
│   │       │   └── queries/      # heatmap, trends, correlation, seasonal
│   │       ├── sqlite/           # OLTP operational DB
│   │       │   ├── mod.rs
│   │       │   ├── connection.rs
│   │       │   ├── migrations.rs
│   │       │   ├── settings.rs   # Key-value settings table
│   │       │   └── queries/      # detections, species, correlation, analytics
│   │       └── resilience.rs     # WAL, backup, integrity, recovery
│   │
│   ├── birdnet-web/              # Web server
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── server.rs         # axum setup, graceful shutdown
│   │       ├── state.rs          # Shared app state (Arc<Mutex>)
│   │       ├── system_info.rs    # CPU/memory/disk via sysinfo
│   │       └── routes/
│   │           ├── mod.rs
│   │           ├── api/          # REST API v2
│   │           │   ├── detections.rs
│   │           │   ├── species.rs
│   │           │   ├── recordings.rs
│   │           │   ├── analytics.rs
│   │           │   └── logs.rs
│   │           ├── pages/        # HTMX server-rendered pages
│   │           │   ├── dashboard.rs
│   │           │   ├── species.rs
│   │           │   ├── heatmap.rs
│   │           │   ├── analytics.rs
│   │           │   └── logs.rs
│   │           └── admin/        # Admin panel
│   │               ├── mod.rs
│   │               ├── settings.rs
│   │               ├── system.rs
│   │               ├── backup.rs
│   │               └── logs.rs
│   │
│   ├── birdnet-integrations/     # External services
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── birdweather.rs    # BirdWeather API client ✅
│   │       ├── apprise.rs        # Apprise notification client ✅
│   │       ├── email.rs          # SMTP email alerts (lettre) ✅
│   │       ├── flickr.rs         # Flickr image caching ✅
│   │       └── rtsp.rs           # RTSP stream management ⚠️
│   │
│   ├── birdnet-behavioral/       # DuckDB behavioral analytics
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── types.rs          # Result/parameter types ✅
│   │       └── queries.rs        # SQL builders ✅
│   │
│   ├── birdnet-timeseries/       # Time series analysis
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── trends.rs         # Trend detection ✅
│   │       ├── moving_average.rs # Rolling window statistics ✅
│   │       └── seasonal.rs       # Seasonal pattern analysis ✅
│   │
│   └── birdnet-migrate/          # BirdNET-Pi migration
│       └── src/
│           ├── lib.rs
│           ├── traits.rs         # Migrator trait ✅
│           ├── report.rs         # MigrationReport types ✅
│           └── birdnet_pi/       # BirdNET-Pi importer
│               ├── mod.rs        # Schema validation + migration ✅
│               └── species_report.rs # Per-species summary ✅
│
├── src/
│   ├── main.rs                   # Binary entry point ✅
│   ├── daemon.rs                 # Detection daemon + event processor ✅
│   └── integrations.rs           # Integration factories ✅
├── tests/                        # Integration tests
└── .github/workflows/            # CI
```

## Crate Responsibilities

| Crate | Sync/Async | Purpose | Status |
|-------|-----------|---------|--------|
| `birdnet-core` | Sync | Audio processing, config, detection types | ✅ Decode/resample; ⚠️ spectrogram |
| `birdnet-db` | Sync | SQLite + DuckDB operations, settings, resilience | ✅ Complete |
| `birdnet-web` | Async | HTMX pages, REST API, SSE, admin panel | ✅ Complete |
| `birdnet-integrations` | Async | BirdWeather, email, Apprise, Flickr | ✅ Complete |
| `birdnet-behavioral` | Sync | DuckDB behavioral analytics types/SQL | ✅ Types + SQL builders |
| `birdnet-timeseries` | Sync | Trend analysis, moving averages, seasonal | ✅ Complete |
| `birdnet-migrate` | Sync | BirdNET-Pi SQLite migration, validation | ✅ Complete |

## Inter-Crate Dependencies

```
main.rs / daemon.rs / integrations.rs
  ├── birdnet-core     (no cross-deps)
  ├── birdnet-db       (no cross-deps)
  ├── birdnet-web ──────→ birdnet-core, birdnet-db
  ├── birdnet-integrations (no cross-deps)
  ├── birdnet-behavioral   (no cross-deps)
  ├── birdnet-timeseries   (no cross-deps)
  └── birdnet-migrate  ──→ birdnet-db
```

The dependency graph is intentionally shallow. Library crates have no
circular dependencies. Only `birdnet-web` depends on `birdnet-core` and
`birdnet-db` for shared types and database access. `birdnet-migrate` depends
on `birdnet-db` for the target database connection.

---

*Last updated: 2026-03-14*

[← Motivation](01-motivation.md) | [Back to Index](../RUST_ARCHITECTURE_PLAN.md) | [Next: Coding Standards →](03-coding-standards.md)
