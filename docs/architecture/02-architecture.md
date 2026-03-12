# Target Architecture

> Single binary design with 5 workspace crates.

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
│   ├── SQLite (operational: detections, real-time queries)
│   ├── DuckDB (analytics: trends, aggregations, behavioral)
│   └── Resilience (WAL, backup, integrity, recovery)
│
├── Web Server (axum)
│   ├── REST API (/api/v2/*)
│   ├── WebSocket (/ws/detections)
│   ├── HTMX partials
│   └── Static files (embedded via rust-embed)
│
├── Integrations
│   ├── BirdWeather (reqwest + retry queue)
│   ├── Notifications (subprocess or native)
│   ├── Image caching (Flickr/Wikipedia)
│   └── RTSP/Icecast
│
└── Operations
    ├── Health monitoring
    ├── Disk management
    ├── Config validation
    └── Graceful shutdown
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
│   │       │   ├── decode.rs     # WAV/FLAC/MP3 via symphonia
│   │       │   ├── resample.rs   # Via rubato
│   │       │   └── spectrogram.rs # Mel spectrogram (pure Rust)
│   │       └── detection/
│   │           ├── mod.rs
│   │           ├── pipeline.rs   # Watch → Analyze → Report
│   │           └── types.rs      # Detection, RecordingFile types
│   │
│   ├── birdnet-db/               # Database layer
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── sqlite.rs         # Operational DB (rusqlite)
│   │       ├── resilience.rs     # WAL, backup, integrity, recovery
│   │       └── migration.rs      # Schema versioning
│   │
│   ├── birdnet-web/              # Web server
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── server.rs         # axum setup, graceful shutdown
│   │       ├── state.rs          # Shared app state (Arc<Mutex>)
│   │       └── routes/
│   │           ├── mod.rs
│   │           ├── detections.rs
│   │           ├── species.rs
│   │           ├── system.rs     # Health, stats
│   │           └── analytics.rs  # DuckDB-powered analytics
│   │
│   ├── birdnet-integrations/     # External services
│   │   └── src/
│   │       ├── lib.rs
│   │       └── birdweather.rs    # BirdWeather API client
│   │
│   └── birdnet-behavioral/       # DuckDB behavioral analytics
│       └── src/
│           ├── lib.rs
│           ├── types.rs          # Result/parameter types
│           └── queries.rs        # SQL builders
│
├── src/
│   └── main.rs                   # Binary entry point
├── tests/                        # Integration tests
├── benches/                      # Benchmarks (criterion)
└── .github/workflows/            # CI
```

## Crate Responsibilities

| Crate | Sync/Async | Purpose | Dependencies |
|-------|-----------|---------|-------------|
| `birdnet-core` | Sync | Audio processing, config, detection types | symphonia, rubato, notify |
| `birdnet-db` | Sync | SQLite operations, resilience, migrations | rusqlite |
| `birdnet-web` | Async | REST API, WebSocket, HTMX serving | axum, tokio, tower |
| `birdnet-integrations` | Async | BirdWeather, notifications | reqwest, tokio |
| `birdnet-behavioral` | Sync | DuckDB behavioral analytics types/SQL | serde (types only) |

## Inter-Crate Dependencies

```
main.rs
  ├── birdnet-core
  ├── birdnet-db
  ├── birdnet-web ──────→ birdnet-core, birdnet-db
  ├── birdnet-integrations
  └── birdnet-behavioral
```

The dependency graph is intentionally shallow. Library crates (`birdnet-core`,
`birdnet-db`, `birdnet-behavioral`) have no cross-dependencies. Only the web
crate depends on core and db for shared types and database access.

---

[← Motivation](01-motivation.md) | [Back to Index](../RUST_ARCHITECTURE_PLAN.md) | [Next: Coding Standards →](03-coding-standards.md)
