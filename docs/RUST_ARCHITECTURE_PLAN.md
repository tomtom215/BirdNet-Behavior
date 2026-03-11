# BirdNET-Pi Rust Architecture Plan

> A phased plan to rewrite BirdNET-Pi's core in Rust for reliability, efficiency,
> and sustainability on resource-constrained field deployments (including solar-powered stations).
>
> **Author:** tomtom215 | **Date:** 2026-03-11

## Table of Contents

- [Motivation](#motivation)
- [Current Architecture Summary](#current-architecture-summary)
- [Target Architecture](#target-architecture)
- [Coding Standards & Conventions](#coding-standards--conventions)
- [Crate Selection](#crate-selection)
- [Phase Plan](#phase-plan)
- [DuckDB as OLAP Analytics Engine](#duckdb-as-olap-analytics-engine)
- [duckdb-behavioral Integration Experiment](#duckdb-behavioral-integration-experiment)
- [Cross-Compilation & Deployment](#cross-compilation--deployment)
- [Migration Strategy](#migration-strategy)
- [Risk Assessment](#risk-assessment)
- [Appendix: Current Python Component Map](#appendix-current-python-component-map)

---

## Motivation

### Why Leave Python

| Problem | Impact on Field Stations |
|---------|------------------------|
| Dependency rot | pip updates break installs; librosa/numpy/scipy version conflicts |
| Memory bloat | Python + librosa + TFLite interpreter: 300-600 MB RSS on a 1 GB Pi |
| GC pauses | Unpredictable latency during real-time analysis |
| No static typing enforcement | Runtime TypeErrors in production after weeks of uptime |
| Startup time | 5-15s cold start importing numpy/scipy/librosa |
| Distribution | Requires virtualenv, pip, system Python matching - fragile on Debian upgrades |

### Why Rust

| Advantage | BirdNET-Pi Benefit |
|-----------|-------------------|
| Zero-cost abstractions | Mel spectrogram computation without runtime overhead |
| Predictable memory | 20-50 MB RSS for entire station binary |
| No GC | Deterministic latency for real-time audio pipeline |
| Single binary | `scp birdnet-pi pi@station:` - done. No pip, no venv, no apt. |
| Cross-compilation | Build for aarch64 on CI, deploy anywhere |
| Fearless concurrency | Safe parallel audio processing and async web serving |
| Long-running stability | No memory leaks from reference cycles, no GIL contention |

### Why Not Go

Go is a reasonable alternative (and tomtom215 has production Go code: `lyrebirdaudio-go`
with Erlang-style supervision trees, Prometheus metrics, and 87% code coverage). However,
Rust wins for BirdNET-Pi:

- **GC pauses** still exist (lower than Python but non-zero; matters for real-time audio)
- **Binary size** is larger (Go embeds runtime; Rust strips to near-C sizes with `strip = true`)
- **Memory usage** is higher (goroutine stacks, GC overhead vs Rust's zero-cost abstractions)
- **Ecosystem for audio/ML** is weaker (no equivalent to symphonia, mel_spec, ort)
- **DuckDB ecosystem** is entirely Rust-based in tomtom215's repos (duckdb-behavioral, quack-rs, mallardmetrics)
- **Language selection pattern**: tomtom215 uses Go for infrastructure/systems tools (audio streaming, device management) and Rust for performance-critical work (analytics engines, FFI, web backends with embedded databases) -- BirdNET-Pi is firmly in the latter category

---

## Current Architecture Summary

### Services (Python + Shell)

```
┌─────────────────────────────────────────────────┐
│ birdnet_recording.sh  │ Audio capture (arecord/ffmpeg/RTSP)
│ birdnet_analysis.py   │ ML inference daemon (inotify → TFLite → DB)
│ web/main.py (FastAPI) │ REST API + WebSocket + HTMX pages
│ disk_check.sh         │ Disk space management (cron)
│ backup_data.sh        │ Backup/restore
└─────────────────────────────────────────────────┘
  Managed by: supervisord
  Served by:  Caddy reverse proxy
  Database:   SQLite (birds.db)
```

### Python Components to Rewrite (~24,000 LOC production, ~19,000 LOC tests)

**Core Pipeline (~2,500 LOC):**

| Component | LOC | Complexity | Dependencies |
|-----------|-----|------------|-------------|
| `birdnet_analysis.py` | 337 | Medium | inotify, threading, requests |
| `utils/analysis.py` | ~300 | High | librosa, numpy, TFLite |
| `utils/models.py` | 262 | High | TFLite runtime, 4 model variants |
| `utils/runtime.py` | 541 | Medium | 3-tier LiteRT abstraction |
| `utils/reporting.py` | 296 | Medium | sqlite3, requests, PIL, sox |
| `utils/config_manager.py` | 569 | Medium | Type-safe config with validation |
| `utils/helpers.py` | 147 | Low | configparser |
| `utils/classes.py` | 84 | Low | datetime, tzlocal |
| `utils/notifications.py` | ~200 | Low | apprise |
| `utils/db.py` | 103 | Low | sqlite3 |

**Web Server (~7,000+ LOC):**

| Component | LOC | Complexity | Dependencies |
|-----------|-----|------------|-------------|
| `web/main.py` | 253 | Medium | FastAPI, uvicorn |
| `web/database.py` | 247 | Medium | aiosqlite |
| `web/db_resilience.py` | 400 | Medium | aiosqlite, sqlite3 |
| `web/images.py` | 643 | Medium | sqlite3, requests |
| `web/websocket.py` | ~200 | Medium | FastAPI WebSocket |
| `web/routers/` (12 files) | ~5,000+ | Medium | FastAPI |

**Shell Scripts (~500 LOC):**

| Script | LOC | Purpose |
|--------|-----|---------|
| `birdnet_recording.sh` | ~200 | ffmpeg/arecord audio capture |
| `disk_check.sh` | 51 | Disk space management |
| `backup_data.sh` | 201 | Backup/restore |

### External Integrations

- **BirdWeather API** - HTTPS POST for soundscapes and detections
- **Apprise** - Multi-channel notifications (email, Telegram, etc.)
- **Flickr/Wikipedia APIs** - Species image caching
- **RTSP streams** - Multi-camera audio via ffmpeg/MediaMTX
- **Icecast** - Live audio streaming

---

## Target Architecture

### Single Binary Design

Inspired by tomtom215's `mallardmetrics` pattern: a single Rust binary that embeds
all functionality, deployed as one file.

```
birdnet-pi (single binary)
├── Core Engine
│   ├── Audio Capture (replaces birdnet_recording.sh)
│   ├── ML Inference (TFLite/ONNX via FFI)
│   ├── Detection Pipeline (inotify → analyze → report)
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
│   ├── Apprise (subprocess or native)
│   ├── Image caching (Flickr/Wikipedia)
│   └── RTSP/Icecast
│
└── Operations
    ├── Health monitoring
    ├── Disk management
    ├── Config validation
    └── Graceful shutdown
```

### Workspace Layout

Following tomtom215's established patterns (quack-rs, duckdb-behavioral):

```
birdnet-pi-rs/
├── Cargo.toml                    # Workspace root
├── crates/
│   ├── birdnet-core/             # Detection pipeline, audio, ML
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── audio/
│   │       │   ├── capture.rs    # Mic/RTSP recording
│   │       │   ├── decode.rs     # WAV/FLAC via symphonia
│   │       │   ├── resample.rs   # Via rubato
│   │       │   └── spectrogram.rs # Mel spectrogram via mel_spec
│   │       ├── inference/
│   │       │   ├── model.rs      # Model loading & lifecycle
│   │       │   ├── tflite.rs     # TFLite FFI bindings
│   │       │   └── onnx.rs       # ONNX Runtime alternative
│   │       ├── detection/
│   │       │   ├── pipeline.rs   # Watch → Analyze → Report
│   │       │   ├── extraction.rs # Audio clip extraction (replaces sox)
│   │       │   └── types.rs      # Detection, ParseFileName equivalents
│   │       └── config.rs         # birdnet.conf parser
│   │
│   ├── birdnet-db/               # Database layer
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── sqlite.rs         # Operational DB (rusqlite)
│   │       ├── duckdb.rs         # Analytics DB
│   │       ├── resilience.rs     # WAL, backup, integrity, recovery
│   │       └── migration.rs      # Schema versioning
│   │
│   ├── birdnet-web/              # Web server
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── server.rs         # axum setup
│   │       ├── routes/
│   │       │   ├── detections.rs
│   │       │   ├── species.rs
│   │       │   ├── system.rs
│   │       │   ├── analytics.rs  # DuckDB-powered analytics
│   │       │   └── pages.rs      # HTMX full pages
│   │       ├── websocket.rs
│   │       └── auth.rs
│   │
│   ├── birdnet-integrations/     # External services
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── birdweather.rs
│   │       ├── apprise.rs
│   │       ├── images.rs         # Flickr/Wikipedia
│   │       └── retry.rs          # Retry queue with offline buffering
│   │
│   └── birdnet-behavioral/       # DuckDB behavioral analytics
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs
│           ├── funnel.rs         # Bird activity funnels
│           ├── retention.rs      # Species return patterns
│           └── sequences.rs      # Song pattern analysis
│
├── src/
│   └── main.rs                   # Binary entry point
├── tests/                        # Integration tests
├── benches/                      # Benchmarks (criterion)
└── .github/workflows/            # CI (matching duckdb-behavioral patterns)
```

---

## Coding Standards & Conventions

Derived from tomtom215's established Rust patterns across duckdb-behavioral,
quack-rs, and mallardmetrics. BirdNET-Pi Rust code MUST follow these conventions.

### Release Profile

```toml
[profile.release]
opt-level = 3
lto = true
codegen-units = 1
panic = "abort"
strip = true
```

### Linting

```toml
[lints.clippy]
all = "warn"
pedantic = "warn"
nursery = "warn"
cargo = "warn"
# Pragmatic allowances
module_name_repetitions = "allow"
must_use_candidate = "allow"
```

### Error Handling

- **Hand-rolled error types** -- no `anyhow` or `thiserror` in library crates
- Struct-based errors with string message and `to_c_string()` for FFI boundaries
- `Result<T, E>` throughout; never panic across FFI or async boundaries
- Application code (birdnet-web) may use `anyhow` for convenience

### Testing Philosophy

- Unit tests within modules (`#[cfg(test)]` pattern)
- **Property-based testing** with `proptest` for data pipeline validation
- **Criterion.rs benchmarks** with HTML reports for performance-critical paths
- E2E tests against real systems (DuckDB CLI, actual WAV files)
- **Mutation testing** via `cargo-mutants` (target: >85% kill rate)
- Coverage tracked via `cargo-tarpaulin` + Codecov
- MSRV explicitly specified and CI-enforced

### CI/CD (13 Jobs, GitHub Actions)

Following duckdb-behavioral's proven 6-workflow pattern:

1. **Quality**: `fmt` → `clippy` → `check` → `doc` (fail fast)
2. **Testing**: `nextest` on Ubuntu + macOS, MSRV verification
3. **Security**: `cargo-deny` supply chain audit, CodeQL static analysis
4. **Compatibility**: SemVer check against main branch
5. **Coverage**: `cargo-tarpaulin` → Codecov
6. **Release**: 4-platform builds with provenance attestation

Action versions pinned by **commit SHA** (not tags) for reproducibility.
Concurrency cancellation for redundant PR runs.

### Dependencies Policy

- **Minimal dependencies in library crates** (quack-rs has only `libduckdb-sys`)
- Practical dependencies in application crate (birdnet-web)
- Exact version pinning for DuckDB compatibility (`=1.4.4`)
- All deps audited via `cargo-deny` (licenses, advisories, sources)

### Async Convention

- **No async in library crates** (birdnet-core, birdnet-db are synchronous)
- **Tokio only in application code** (birdnet-web uses `tokio` with full features)
- Blocking operations via `tokio::task::spawn_blocking` (DB queries, file I/O, inference)

---

## Crate Selection

### Core Dependencies

| Purpose | Crate | Maturity | Notes |
|---------|-------|----------|-------|
| Async runtime | `tokio` | Production | Full-featured, well-tested on ARM |
| Web framework | `axum` | Production | Tower-based, lighter than actix-web |
| HTTP client | `reqwest` | Production | BirdWeather, Flickr, Wikipedia API calls |
| SQLite | `rusqlite` | Production | With `bundled` feature for zero system deps |
| DuckDB | `duckdb` | Production | tomtom215's own fork/experience |
| Audio decode | `symphonia` | Production | Pure Rust, no system deps |
| Resampling | `rubato` | Production | High-quality async resampling |
| Mel spectrogram | `mel_spec` | Maturing | Aligned with librosa output |
| File watching | `notify` | Production | Cross-platform inotify wrapper |
| Config parsing | `toml` or custom | Production | For birdnet.conf (INI-style) |
| Serialization | `serde` + `serde_json` | Production | Standard |
| Logging | `tracing` | Production | Structured, async-aware |
| CLI | `clap` | Production | Argument parsing |
| Error handling | Hand-rolled (libs) / `anyhow` (app) | Production | Following tomtom215 patterns: no thiserror in libs |
| Image processing | `image` | Production | Spectrogram PNG generation |
| Template engine | `askama` or `minijinja` | Production | HTMX template rendering |
| Embedded assets | `rust-embed` | Production | Static files in binary |

### ML Inference Options

| Option | Crate | Status | Cross-compile | Recommendation |
|--------|-------|--------|---------------|---------------|
| TFLite via FFI | `tflite` v0.9.8 | C++ API bindings | **Hard** (requires Bazel + full TF source) | Avoid |
| TFLite C API | `tflitec` | Pinned to TF v2.9.1 | Medium | Avoid (outdated) |
| ONNX Runtime | [`ort`](https://github.com/pykeio/ort) v2.0 | **Production** (wraps ONNX RT 1.24) | Medium (pre-built aarch64 binaries) | **Primary choice** |
| Tract (pure Rust) | `tract-tflite` | Good (Sonos) | **Trivial** (pure Rust) | Fallback / long-term |

**Recommendation:** **`ort`** (ONNX Runtime). Used by SurrealDB, Google Magika, Bloop.

Convert BirdNET models:
```bash
pip install tf2onnx onnxruntime
python -m tf2onnx.convert --tflite BirdNET_model.tflite --output BirdNET_model.onnx --opset 18
```

The `ort` crate provides:
- ARM64 with NEON acceleration (XNNPACK backend)
- `half` feature flag for FP16 tensor support (critical for BirdNET FP16 models)
- Quantized models (INT8) for further Pi optimization
- Thread pool configuration for constrained environments
- Pre-built aarch64 binaries from Microsoft (auto-downloaded by `ort`)
- Model loading from file or embedded bytes

**Fallback:** `tract-tflite` is pure Rust (trivial cross-compilation) but FP16 operator
coverage needs validation against the actual BirdNET model. Worth testing as the
long-term zero-dependency goal.

### Audio Pipeline (Pure Rust, Zero C Dependencies)

| Task | Crate | Downloads | Notes |
|------|-------|-----------|-------|
| Decode WAV/FLAC/MP3 | [`symphonia`](https://github.com/pdeljanov/Symphonia) | 3.2M+ | Pure Rust, royalty-free codecs by default |
| Resampling | [`rubato`](https://lib.rs/crates/rubato) | Active (Jan 2026) | Async resampling designed for audio |
| Mel spectrogram | [`mel_spec`](https://crates.io/crates/mel_spec) | Moderate | **Aligned to librosa reference output** -- critical for model accuracy |

**Key insight:** `mel_spec` produces output aligned with librosa, PyTorch, and whisper.cpp
reference implementations. Since BirdNET models were trained on librosa-generated spectrograms,
this numerical equivalence is critical. Encode rate: 6.4KB/sec (80 x 2 bytes x 40 frames).

The entire audio pipeline (symphonia + rubato + mel_spec) cross-compiles trivially to
aarch64 with no system dependencies.

---

## Phase Plan

### Phase 0: Scaffolding (1-2 days)

- [ ] Initialize Cargo workspace with crate structure above
- [ ] Set up CI matching duckdb-behavioral patterns:
  - Cross-compilation for `aarch64-unknown-linux-gnu`
  - Clippy + rustfmt enforcement
  - Test matrix (x86_64 + aarch64)
  - Release builds with provenance attestation
- [ ] Set up integration test framework
- [ ] Create `birdnet.conf` parser in Rust (INI-style with PHP quote stripping)
- [ ] Verify cross-compilation toolchain for Raspberry Pi

### Phase 1: Data Layer (3-5 days)

**Goal:** Replace the entire database layer with Rust, running alongside Python.

- [ ] `birdnet-db` crate with `rusqlite`:
  - WAL mode enforcement
  - Connection pooling (r2d2 or deadpool)
  - Integrity checking and backup (SQLite backup API)
  - Corruption recovery
  - Schema migration framework
- [ ] DuckDB integration:
  - Attach SQLite for live ETL: `ATTACH 'birds.db' AS sqlite_db (TYPE SQLITE)`
  - Analytics views and materialized aggregations
  - Time-series detection analysis
- [ ] Write detection INSERT path (replaces `reporting.py:write_to_db`)
- [ ] Write all read queries (replaces `utils/db.py`)
- [ ] **Test:** Run Rust DB layer writing detections while Python reads (and vice versa)
  - Both use WAL mode, so concurrent access is safe

### Phase 2: Audio Pipeline (5-7 days)

**Goal:** Replace librosa/soundfile/sox with pure Rust audio processing.

- [ ] WAV decoding via `symphonia`
- [ ] Resampling via `rubato` (48kHz → model sample rate)
- [ ] Mel spectrogram via `mel_spec`:
  - Validate output matches librosa within tolerance (critical for model accuracy)
  - Benchmark against Python: expect 5-10x speedup
- [ ] Audio extraction (replaces `sox trim`):
  - Read WAV segment, write to output format
  - FLAC encoding for BirdWeather uploads
- [ ] Spectrogram PNG generation via `image` crate (replaces sox + PIL)
- [ ] **Test:** Feed identical WAV files through Python and Rust pipelines,
  compare mel spectrogram matrices (must be within 1e-4 tolerance)

### Phase 3: ML Inference (3-5 days)

**Goal:** Run BirdNET model inference in Rust.

- [ ] Convert BirdNET TFLite FP16 model to ONNX:
  ```bash
  python -m tf2onnx.convert --tflite model.tflite --output model.onnx
  ```
- [ ] Integrate `ort` crate for ONNX Runtime:
  - Load model
  - Run inference on mel spectrogram input
  - Parse output labels and confidence scores
- [ ] Validate predictions match Python output (same inputs → same top-5 species)
- [ ] Benchmark: inference latency on Pi 4/5 (target: <1s per 3-second chunk)
- [ ] Model hot-reload support (for OTA model updates)

### Phase 4: Detection Daemon (3-5 days)

**Goal:** Replace `birdnet_analysis.py` entirely.

- [ ] File watcher via `notify` crate (replaces inotify adapter)
- [ ] Detection pipeline: watch → decode → spectrogram → infer → report
- [ ] Reporting:
  - Write to SQLite (from Phase 1)
  - Write to BirdDB.txt CSV
  - Write JSON detection file
  - Notify web server via HTTP POST
- [ ] Graceful shutdown with in-flight analysis completion
- [ ] Memory-bounded: no accumulation over days/weeks
- [ ] **Test:** Run as systemd service, process real recordings, compare output

### Phase 5: Web Server (5-7 days)

**Goal:** Replace FastAPI with axum.

- [ ] axum server setup with Tower middleware:
  - CORS
  - Request logging via `tracing`
  - Authentication (matching current Caddy basic auth)
- [ ] Port all REST API endpoints:
  - `/api/v2/detections/*`
  - `/api/v2/species/*`
  - `/api/v2/system/*` (health, services, diagnostics)
  - `/api/v2/analytics/*` (DuckDB-powered)
  - `/api/v2/export/*`
- [ ] WebSocket endpoint for live detections (tokio broadcast channel)
- [ ] HTMX partial rendering via askama/minijinja templates
- [ ] Static file serving (embedded in binary via rust-embed)
- [ ] **Test:** Run both Python and Rust servers, compare API responses

### Phase 6: Integrations (2-3 days)

- [ ] BirdWeather client with retry queue and offline buffering
- [ ] Apprise notifications (via subprocess initially, native later)
- [ ] Flickr/Wikipedia image caching (SQLite-backed)
- [ ] RTSP stream management
- [ ] Heartbeat monitoring

### Phase 7: Audio Capture (2-3 days)

- [ ] Replace `birdnet_recording.sh`:
  - ALSA capture via `cpal` or subprocess `arecord`
  - RTSP capture via subprocess `ffmpeg`
  - Gap detection and alerting
  - Automatic reconnection
- [ ] Disk space management (replaces `disk_check.sh`)

### Phase 8: Single Binary Assembly (1-2 days)

- [ ] Unified `main.rs` that starts all subsystems
- [ ] Signal handling (SIGTERM, SIGINT) with graceful shutdown
- [ ] systemd service file
- [ ] Config validation on startup with fail-fast
- [ ] Health check endpoint for monitoring

---

## DuckDB as OLAP Analytics Engine

### Dual-Database Architecture

```
SQLite (OLTP)                    DuckDB (OLAP)
─────────────                    ──────────────
Real-time writes                 Analytical queries
Detection inserts                Trend analysis
Live detection feed              Species aggregations
Web API read queries             Confidence distributions
Small, fast, embedded            Columnar, vectorized
WAL for crash safety             Append-only Parquet backing
```

### Why DuckDB for Analytics

SQLite is excellent for OLTP (the operational detection writes and web queries)
but struggles with analytical workloads on large datasets:

| Query Type | SQLite | DuckDB |
|------------|--------|--------|
| `SELECT * WHERE Date = today` | Fast | Fast |
| `GROUP BY species ORDER BY COUNT(*)` | Slow at 1M+ rows | Vectorized, instant |
| `Window functions over time series` | Possible but slow | Native, optimized |
| `Confidence distribution histogram` | Table scan | Columnar scan |
| `Year-over-year species comparison` | Minutes | Seconds |

### ETL Pipeline

```rust
// Periodic sync from SQLite → DuckDB (every N minutes)
async fn sync_detections(sqlite: &SqlitePool, duckdb: &DuckDbConn) {
    // DuckDB can directly attach and query SQLite files
    duckdb.execute("ATTACH 'birds.db' AS sqlite_db (TYPE SQLITE)")?;

    // Incremental sync: only new rows since last sync
    duckdb.execute("
        INSERT INTO detections
        SELECT * FROM sqlite_db.detections
        WHERE Date > ? OR (Date = ? AND Time > ?)
    ", &[&last_sync_date, &last_sync_date, &last_sync_time])?;

    duckdb.execute("DETACH sqlite_db")?;
}
```

### Analytics API Endpoints (DuckDB-powered)

```
GET /api/v2/analytics/trends
  → Detections per hour/day/week with moving averages

GET /api/v2/analytics/species-activity
  → Activity heatmap (species × hour-of-day)

GET /api/v2/analytics/confidence-distribution
  → Histogram of confidence scores by species

GET /api/v2/analytics/seasonal-patterns
  → Species arrival/departure dates across years

GET /api/v2/analytics/site-comparison
  → Multi-station comparison (for fleet deployments)
```

---

## duckdb-behavioral Integration Experiment

### Concept

Apply tomtom215's [duckdb-behavioral](https://github.com/tomtom215/duckdb-behavioral)
extension to bird detection data. The extension provides ClickHouse-inspired
behavioral analytics functions:

- **`sessionize`** - Group detections into activity sessions
- **`retention`** - Track species return patterns
- **`window_funnel`** - Analyze sequences of bird activity
- **`sequence_match` / `sequence_count`** - Find patterns in detection sequences
- **`sequence_next_node`** - Predict likely next species after a detection

### Bird Behavior Analytics Queries

#### 1. Activity Sessionization

Group continuous bird activity into sessions (gap > 30 minutes = new session):

```sql
LOAD behavioral;

SELECT
    Com_Name,
    sessionize(detection_timestamp, INTERVAL '30 MINUTE')
        OVER (PARTITION BY Sci_Name ORDER BY detection_timestamp)
        AS session_id,
    COUNT(*) as detections_in_session,
    MIN(detection_timestamp) as session_start,
    MAX(detection_timestamp) as session_end
FROM detections_ts
GROUP BY Com_Name, session_id
ORDER BY session_start DESC;
```

**Use case:** Understand when birds are actively vocalizing vs. just passing through.
A dawn chorus session might show 50 detections in 30 minutes, while a territorial
call is 3 detections over 5 minutes.

#### 2. Species Retention Analysis

Which species keep coming back day after day?

```sql
SELECT
    Com_Name,
    retention(detection_date, [1, 2, 3, 7, 14, 30]) AS retention_rates
FROM (
    SELECT DISTINCT Com_Name, CAST(Date AS DATE) AS detection_date
    FROM detections
)
GROUP BY Com_Name
ORDER BY retention_rates[1] DESC;
```

**Use case:** Distinguish resident species (high retention) from migrants (appear
for a few days then gone) or rarities (single-day events).

#### 3. Dawn Chorus Funnel

Do species follow a predictable sequence at dawn?

```sql
SELECT window_funnel(
    INTERVAL '2 HOUR',
    detection_timestamp,
    [
        Com_Name = 'European Robin',      -- Step 1: Robin starts
        Com_Name = 'Eurasian Blackbird',   -- Step 2: Blackbird joins
        Com_Name = 'Song Thrush',          -- Step 3: Thrush follows
        Com_Name = 'Eurasian Wren',        -- Step 4: Wren adds
        Com_Name = 'Great Tit'             -- Step 5: Great Tit completes
    ]
) AS dawn_chorus_stage
FROM detections_ts
WHERE EXTRACT(HOUR FROM detection_timestamp) BETWEEN 4 AND 8
GROUP BY CAST(detection_timestamp AS DATE);
```

**Use case:** Validate the well-known dawn chorus ordering. Each morning, how many
"steps" of the expected chorus sequence actually occur?

#### 4. Sequence Pattern Matching

Find days with specific bird activity patterns:

```sql
SELECT
    CAST(detection_timestamp AS DATE) AS detection_date,
    sequence_match(
        '(?1).*(?2).*(?3)',  -- Robin followed by Blackbird followed by Thrush
        detection_timestamp,
        [
            Com_Name = 'European Robin',
            Com_Name = 'Eurasian Blackbird',
            Com_Name = 'Song Thrush'
        ]
    ) AS pattern_matched
FROM detections_ts
GROUP BY detection_date
HAVING pattern_matched = true;
```

**Use case:** Ecological research - do certain species always appear in sequence?
Does species A arriving predict species B within N hours?

#### 5. Predictive: What Species Comes Next?

After detecting a Robin, what typically follows?

```sql
SELECT sequence_next_node(
    detection_timestamp,
    INTERVAL '1 HOUR',
    Com_Name = 'European Robin',  -- Trigger event
    1,                             -- Level (1 = immediate next)
    'strict'                       -- Mode
) AS next_species,
COUNT(*) as frequency
FROM detections_ts
GROUP BY next_species
ORDER BY frequency DESC
LIMIT 10;
```

**Use case:** Build a "what to expect next" prediction feature for the web UI.
After hearing a Robin, the system can suggest "Blackbird likely in next 15 minutes (82%)".

### Implementation Plan

1. **Data Preparation:**
   - Create `detections_ts` view with proper TIMESTAMP column:
     ```sql
     CREATE VIEW detections_ts AS
     SELECT *, CAST(Date || ' ' || Time AS TIMESTAMP) AS detection_timestamp
     FROM detections;
     ```

2. **Extension Loading:**
   - Bundle `duckdb-behavioral` with the Rust binary
   - Or install from community: `INSTALL behavioral FROM community; LOAD behavioral;`

3. **API Endpoints:**
   ```
   GET /api/v2/analytics/sessions?species=...&gap=30m
   GET /api/v2/analytics/retention?species=...&periods=1,7,30
   GET /api/v2/analytics/funnel?sequence=Robin,Blackbird,Thrush&window=2h
   GET /api/v2/analytics/patterns?regex=(?1).*(?2)&conditions=...
   GET /api/v2/analytics/next-species?after=Robin&window=1h
   ```

4. **Web UI:**
   - Activity session timeline visualization
   - Species retention heatmap
   - Dawn chorus funnel chart
   - "What's coming next?" prediction widget

---

## Cross-Compilation & Deployment

### Cross-Compile Difficulty by Dependency

| Library | Cross-compile | Notes |
|---------|---------------|-------|
| symphonia, rubato, mel_spec | **None** | Pure Rust |
| axum, notify, serde | **None** | Pure Rust |
| rusqlite (bundled) | **None** | Bundles SQLite C source |
| `ort` (ONNX Runtime) | **Medium** | Pre-built aarch64 binaries auto-downloaded by crate |
| `duckdb` | **Medium-High** | Needs C++ cross-toolchain; custom `cross` Docker image |
| `tflite` (if used) | **High** | Requires Bazel + full TF source -- avoid |

**Approach:** `cross-rs` with custom Docker image extending the base with aarch64 DuckDB dev libs.
Alternative: `cargo-zigbuild` using Zig's cross-linker (simpler but may struggle with DuckDB C++).
Fallback: Native compilation on Pi 5 (slowest but zero cross-compile complexity).

### CI/CD Pipeline

Following duckdb-behavioral's proven CI patterns (action versions pinned by commit SHA):

```yaml
# .github/workflows/release.yml
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
          - target: armv7-unknown-linux-gnueabihf  # Pi Zero 2W (32-bit compat)
            os: ubuntu-latest

    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}
      - uses: cross-rs/cross-action@v1  # Cross-compilation
        with:
          command: build
          args: --release --target ${{ matrix.target }}
      - name: Upload artifact
        uses: actions/upload-artifact@v4
        with:
          name: birdnet-pi-${{ matrix.target }}
          path: target/${{ matrix.target }}/release/birdnet-pi
```

### Deployment

```bash
# On the Pi:
curl -L https://github.com/tomtom215/birdnet-pi-rs/releases/latest/download/birdnet-pi-aarch64 \
  -o /usr/local/bin/birdnet-pi
chmod +x /usr/local/bin/birdnet-pi
systemctl restart birdnet-pi

# That's it. No pip. No venv. No apt dependencies. No broken numpy.
```

### Resource Expectations

| Metric | Python (current) | Rust (target) |
|--------|-----------------|---------------|
| Binary/install size | ~500 MB (venv + deps) | ~15-30 MB (single binary) |
| RSS memory (idle) | ~200 MB | ~10-20 MB |
| RSS memory (analyzing) | ~400-600 MB | ~30-50 MB |
| Cold start time | 5-15 seconds | <1 second |
| Inference latency (3s clip) | ~1-2 seconds | ~0.5-1 second |
| Disk write durability | WAL (just added) | WAL + fsync control |

---

## Migration Strategy

### Parallel Running Period

During migration, both Python and Rust components can coexist:

```
Phase 1-2: Python analysis daemon + Rust DB layer
           (Rust writes to SQLite, Python reads - WAL makes this safe)

Phase 3-4: Rust analysis daemon + Rust DB layer
           Python web server still running

Phase 5:   Rust analysis daemon + Rust web server
           Python fully removed

Phase 6+:  Single binary, Python gone
```

### Backwards Compatibility

- Same SQLite database schema (no migration needed)
- Same birdnet.conf format (Rust parser handles PHP-style quotes)
- Same API endpoints (drop-in replacement for FastAPI)
- Same systemd service names
- Same Caddy reverse proxy config

### Rollback Plan

At any phase, can revert to Python:
1. Stop Rust binary
2. Start Python services
3. Both use the same database and config

---

## Risk Assessment

| Risk | Severity | Mitigation |
|------|----------|------------|
| ONNX model conversion loses accuracy | High | Validate predictions match Python ±0.01 confidence |
| Mel spectrogram differences affect inference | High | Bit-accurate comparison tests with librosa |
| DuckDB ARM64 performance untested | Medium | Benchmark early in Phase 1; fallback to SQLite-only |
| Cross-compilation with native libs (ONNX RT) | Medium | Use `cross` with custom Docker images |
| RTSP/audio capture complexity | Low | Keep ffmpeg subprocess; don't rewrite in pure Rust |
| Web template migration effort | Low | Askama is similar to Jinja2; mechanical translation |
| Community adoption | Low | Ship as optional binary alongside existing Python install |

---

## Appendix: Current Python Component Map

### Entry Points

| File | Type | Systemd Service |
|------|------|-----------------|
| `scripts/birdnet_analysis.py` | Python daemon | `birdnet_analysis.service` |
| `scripts/birdnet_recording.sh` | Bash daemon | `birdnet_recording.service` |
| `scripts/web/main.py` | FastAPI server | `birdnet_web.service` |
| `scripts/disk_check.sh` | Cron job | crontab |
| `scripts/backup_data.sh` | Manual script | - |

### Database Access Points

| File | Operations | Library |
|------|-----------|---------|
| `scripts/utils/reporting.py` | INSERT detections | sqlite3 (sync) |
| `scripts/utils/db.py` | SELECT queries | sqlite3 (sync, cached conn) |
| `scripts/web/database.py` | All operations | aiosqlite (async) |
| `scripts/web/db_resilience.py` | WAL, backup, integrity | aiosqlite + sqlite3 |
| `scripts/web/images.py` | Image cache CRUD | sqlite3 (sync) |
| `scripts/web/routers/*.py` | Via database.py | aiosqlite (async) |

### Audio Pipeline

```
Microphone/RTSP → arecord/ffmpeg → WAV files in StreamData/
                                        │
                    inotify watches ─────┘
                                        │
                    librosa.load() → numpy array
                                        │
                    TFLite interpreter → detections
                                        │
                    sox trim → extracted clips
                    sox spectrogram → PNG images
                                        │
                    SQLite INSERT + BirdWeather POST + Apprise notify
```

### ML Inference Chain

```python
# scripts/utils/analysis.py + utils/models.py
1. librosa.load(wav_file, sr=48000)           # Decode + resample (mono)
2. Split into 3-second chunks with overlap    # (5s for Perch model)
3. Pad short chunks with zeros
4. TFLite interpreter.set_tensor(input)       # Audio data as float32
5. Optional: set metadata tensor              # (lat, lon, week) for V2.4
6. TFLite interpreter.invoke()                # Run inference
7. interpreter.get_tensor(output)             # Get logits
8. sigmoid(sensitivity * logits)              # Apply sensitivity scaling
9. Top-10 species per chunk                   # With confidence scores
10. Filter by confidence threshold
11. Optional: metadata model filters rare species
```

**Model Variants (4):**
- BirdNET V1: 6K species, metadata input
- BirdNET V2.4 FP16: 6K species, separate metadata model
- BirdNET-Go v20250916: Extends V2.4
- Perch V2: Google's model, 5-second chunks, 32kHz sample rate

### IPC & Communication

| Channel | From → To | Mechanism |
|---------|-----------|-----------|
| New WAV file | Recording → Analysis | File system (inotify IN_CLOSE_WRITE) |
| Detection data | Analysis → Web | HTTP POST to `localhost:8502/api/v2/internal/notify` |
| Live updates | Web → Clients | WebSocket broadcast (`/ws/detections`) |
| Status | Analysis → Web | File: `analyzing_now.txt` |
| CSV backup | Analysis → Disk | Append to `BirdDB.txt` |
| Config | Disk → All services | `/etc/birdnet/birdnet.conf` (read on demand) |

---

### tomtom215 Rust Ecosystem (Reference)

| Repository | Purpose | Relevance to BirdNET-Pi |
|---|---|---|
| [duckdb-behavioral](https://github.com/tomtom215/duckdb-behavioral) | ClickHouse-inspired analytics (7 functions, 453 tests) | Bird activity analytics |
| [quack-rs](https://github.com/tomtom215/quack-rs) | SDK for DuckDB Rust extensions | Extension loading infrastructure |
| [mallardmetrics](https://github.com/tomtom215/mallardmetrics) | Single-binary web analytics (axum + DuckDB) | Architecture template |
| [duckdb-rs](https://github.com/tomtom215/duckdb-rs) | Fork of DuckDB Rust bindings | Direct dependency |
| [LyreBirdAudio](https://github.com/tomtom215/LyreBirdAudio) | RTSP audio streaming (12 stars) | Audio capture patterns |
| [lyrebirdaudio-go](https://github.com/tomtom215/lyrebirdaudio-go) | Go: Erlang-style supervision, Prometheus | Supervision/monitoring patterns |

---

*This document is a living plan. Update as research progresses and implementation validates assumptions.*
