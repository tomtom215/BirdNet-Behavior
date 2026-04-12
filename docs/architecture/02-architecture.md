# Target Architecture

> Single binary design with eight workspace crates.

## Table of Contents

- [Single Binary Design](#single-binary-design)
- [Workspace Layout](#workspace-layout)
- [Crate Responsibilities](#crate-responsibilities)
- [Inter-Crate Dependencies](#inter-crate-dependencies)

---

## Single Binary Design

BirdNet-Behavior ships as one Rust binary that embeds every subsystem —
audio capture, inference, storage, HTTP server, integrations, scheduler.
There are no helper processes, no interpreter, no sidecar services.

```
birdnet-behavior (single binary)
├── Core Engine
│   ├── Audio Capture          ALSA / PulseAudio / PipeWire / RTSP
│   ├── ML Inference           ONNX Runtime via the `ort` crate
│   ├── Detection Pipeline     notify → decode → resample → infer → report
│   └── Audio Processing       symphonia, rubato, mel spectrogram
│
├── Data Layer
│   ├── SQLite (OLTP)          Detections, settings, live queries
│   ├── DuckDB (OLAP)          Behavioral + time-series analytics (optional)
│   └── Resilience             WAL, backup, integrity check, recovery
│
├── Web Server (axum)
│   ├── REST API               /api/v2/*
│   ├── WebSocket              Live detection stream, live spectrogram
│   ├── Server-Sent Events     Live logs, detection feed
│   ├── HTMX pages             Dashboard, species, heatmap, analytics
│   └── Admin panel            Settings, system controls, backups, logs
│
├── Integrations
│   ├── BirdWeather            Detection + soundscape upload
│   ├── Apprise                80+ notification channels
│   ├── Email (lettre)         SMTP / STARTTLS, per-species cooldown
│   ├── MQTT                   Pure-Rust 3.1.1 publisher + HA discovery
│   └── Species images         Wikipedia cache
│
├── Analytics
│   ├── Behavioral             Sessionize, retention, funnel, phenology
│   └── Time series            Activity, diversity, trend, peak, gap
│
└── Migration
    └── BirdNET-Pi import      Schema detection, validation, transactional import
```

## Workspace Layout

```
BirdNet-Behavior/
├── Cargo.toml                      # Workspace root
├── src/                            # Binary entry point and application glue
│   ├── main.rs                     # Startup, CLI parse, service wiring
│   ├── cli.rs                      # clap argument definitions
│   ├── daemon.rs                   # Detection event processor
│   ├── capture.rs                  # Audio capture subprocess lifecycle
│   ├── integrations.rs             # Integration factories
│   ├── helpers.rs                  # Disk manager, mDNS, init helpers
│   └── weekly_report.rs            # Weekly report scheduler
│
├── crates/
│   ├── birdnet-core/               # Audio, detection pipeline, inference
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── config.rs           # birdnet.conf parser (INI)
│   │       ├── i18n.rs             # Species name translation
│   │       ├── audio/
│   │       │   ├── capture/        # arecord / ffmpeg / tmpfs / disk manager
│   │       │   ├── decode.rs       # symphonia WAV/FLAC/MP3
│   │       │   ├── resample.rs     # rubato polynomial resampler
│   │       │   ├── extraction/     # Per-detection WAV extraction + metadata
│   │       │   ├── quality/        # SNR, flatness, rain/wind detection
│   │       │   └── spectrogram/    # Mel spectrogram + live broadcast
│   │       ├── detection/
│   │       │   ├── pipeline.rs     # Chunking + inference orchestration
│   │       │   ├── daemon.rs       # File-watcher event loop
│   │       │   ├── privacy.rs      # Human-voice suppression
│   │       │   └── types.rs
│   │       └── inference/
│   │           ├── model.rs        # ort session wrapper
│   │           ├── labels.rs       # BirdNET label parser
│   │           └── species_filter.rs
│   │
│   ├── birdnet-db/                 # Database layer
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── sqlite/             # Connection, queries, types
│   │       ├── migration.rs        # Schema migrations (idempotent)
│   │       ├── resilience.rs       # Backup, restore, integrity check
│   │       ├── settings.rs         # Key-value settings store
│   │       ├── alert_rules.rs      # Detection-triggered actions
│   │       └── notifications.rs    # Notification log and stats
│   │
│   ├── birdnet-web/                # Web server
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── server.rs           # axum setup, graceful shutdown
│   │       ├── state.rs            # Shared application state
│   │       ├── auth.rs             # HTTP Basic Auth
│   │       ├── rate_limit.rs       # Per-IP token-bucket rate limiter
│   │       ├── system_info.rs      # CPU / memory / temperature
│   │       └── routes/             # REST API, HTMX pages, admin panel
│   │
│   ├── birdnet-integrations/       # External services
│   │   └── src/
│   │       ├── birdweather.rs
│   │       ├── apprise.rs
│   │       ├── email/              # SMTP via lettre + rustls
│   │       ├── species_images/     # Wikipedia image cache
│   │       ├── mqtt/               # Pure-Rust MQTT 3.1.1 + HA discovery
│   │       ├── auto_update.rs      # GitHub Releases update
│   │       ├── heartbeat.rs
│   │       ├── notification.rs     # Template rendering
│   │       └── weekly_report.rs
│   │
│   ├── birdnet-behavioral/         # DuckDB behavioral analytics
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── types.rs
│   │       ├── queries.rs          # SQL builders
│   │       ├── connection/         # DuckDB connection and sync
│   │       └── phenology/          # Timing, abundance, migration windows
│   │
│   ├── birdnet-timeseries/         # Time-series analytics
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── queries/            # activity, diversity, trend, peak, gap
│   │       ├── executor/           # Query execution
│   │       ├── window/             # tumbling, sliding, hopping, session
│   │       └── types/
│   │
│   ├── birdnet-migrate/            # BirdNET-Pi migration
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── traits.rs           # Migrator, Validator, SchemaDetector
│   │       ├── schema.rs           # SQLite + CSV schema detection
│   │       ├── progress.rs         # Thread-safe progress handle
│   │       └── birdnet_pi/         # Importer, validator, species report
│   │
│   └── birdnet-scheduler/          # Recording schedule
│       └── src/
│           ├── lib.rs
│           ├── solar.rs            # NOAA/Meeus sunrise/sunset
│           ├── schedule.rs
│           ├── window.rs
│           └── inhibit.rs          # Night-inhibit logic
│
├── docs/                           # Architecture documentation
├── tests/                          # Integration tests
└── .github/workflows/              # CI/CD pipelines
```

## Crate Responsibilities

| Crate | Sync/Async | Purpose |
|-------|-----------|---------|
| `birdnet-core` | Sync | Audio processing, detection pipeline, ML inference |
| `birdnet-db` | Sync | SQLite operations, settings, resilience, alert rules |
| `birdnet-web` | Async | REST API, WebSocket, HTMX pages, admin panel |
| `birdnet-integrations` | Async | BirdWeather, Apprise, email, Wikipedia, MQTT |
| `birdnet-behavioral` | Sync | DuckDB behavioral analytics types and SQL builders |
| `birdnet-timeseries` | Sync | Time-series query builders and executors |
| `birdnet-migrate` | Sync | BirdNET-Pi schema detection and transactional import |
| `birdnet-scheduler` | Sync | Solar calculations and recording window scheduling |

Library crates are synchronous by design. Async is confined to `birdnet-web`
and the binary itself, so library code can be exercised in tests without an
async runtime.

## Inter-Crate Dependencies

```
main.rs / daemon.rs / integrations.rs
  ├── birdnet-core         (no cross-deps)
  ├── birdnet-db           (no cross-deps)
  ├── birdnet-scheduler    (no cross-deps)
  ├── birdnet-integrations (no cross-deps)
  ├── birdnet-behavioral   (no cross-deps)
  ├── birdnet-timeseries   (no cross-deps)
  ├── birdnet-migrate ───→ birdnet-db
  └── birdnet-web ───────→ birdnet-core, birdnet-db, birdnet-integrations,
                            birdnet-migrate, birdnet-behavioral, birdnet-timeseries
```

The graph is intentionally shallow. Library crates have no circular
dependencies. Only `birdnet-web` pulls in multiple sibling crates — it is
the composition point for HTTP-accessible functionality. `birdnet-migrate`
depends on `birdnet-db` solely for the target database connection type.

---

[← Motivation](01-motivation.md) | [Back to Index](../RUST_ARCHITECTURE_PLAN.md) | [Next: Coding Standards →](03-coding-standards.md)
