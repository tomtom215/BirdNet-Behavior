# Implementation Status

> Current state of the Rust implementation as of March 2026.

## Summary

| Phase | Description | Status | Completion |
|-------|------------|--------|------------|
| 0 | Scaffolding | **Complete** | 100% |
| 1 | Data Layer | **Complete** | 100% |
| 2 | Audio Pipeline | **Complete** | 100% |
| 3 | ML Inference | **Complete** | 100% |
| 4 | Detection Daemon | **Complete** | 100% |
| 5 | Web Server | **Complete** | 95% |
| 6 | Integrations | **Partial** | 30% |
| 7 | Audio Capture | **Complete** | 100% |
| 8 | Assembly | **Complete** | 95% |

## Detailed Status by Crate

### birdnet-core

| Module | File | Status | Notes |
|--------|------|--------|-------|
| Config parser | `config.rs` | **Complete** | INI parsing, PHP quote stripping, full tests |
| Audio decode | `audio/decode.rs` | **Complete** | symphonia WAV/FLAC/MP3, mono downmix |
| Audio resample | `audio/resample.rs` | **Complete** | rubato polynomial resampler, chunked processing |
| Mel spectrogram | `audio/spectrogram.rs` | **Complete** | Pure Rust, realfft, librosa-compatible, 128 mel bands |
| Audio capture | `audio/capture.rs` | **Complete** | Subprocess management for arecord/ffmpeg |
| Detection types | `detection/types.rs` | **Complete** | Detection struct, RecordingFile parser, serde |
| Detection pipeline | `detection/pipeline.rs` | **Complete** | File watching, chunking, spectrogram prep |
| Detection daemon | `detection/daemon.rs` | **Complete** | File watcher, inference loop, event broadcasting |
| Inference labels | `inference/labels.rs` | **Complete** | BirdNET label format parser, lookup by sci/common name |
| Inference model | `inference/model.rs` | **Complete** | tract-onnx ONNX inference, sigmoid/softmax, multi-model |

### birdnet-db

| Module | File | Status | Notes |
|--------|------|--------|-------|
| SQLite operations | `sqlite.rs` | **Complete** | WAL, CRUD, aggregation queries, full tests |
| Resilience | `resilience.rs` | **Complete** | Backup, restore, integrity, auto-recovery |
| Migrations | `migration.rs` | **Complete** | 3 migrations, idempotent, version tracking |

### birdnet-web

| Module | File | Status | Notes |
|--------|------|--------|-------|
| Server setup | `server.rs` | **Complete** | axum, middleware, graceful shutdown |
| App state | `state.rs` | **Complete** | Arc<Mutex>, auto-migration, DuckDB analytics, detection broadcast |
| Detection routes | `routes/detections.rs` | **Complete** | GET by date, recent, with limits, proper HTTP status codes |
| Species routes | `routes/species.rs` | **Complete** | Top species, hourly activity, proper HTTP status codes |
| System routes | `routes/system.rs` | **Complete** | Health (503 on degraded), stats, API info |
| Analytics routes | `routes/analytics.rs` | **Complete** | Sessions, retention, funnel, next-species (DuckDB-backed when analytics feature enabled) |
| WebSocket | `routes/websocket.rs` | **Complete** | Live detection streaming, broadcast, ping/pong |
| HTMX pages | `routes/pages.rs` | **Complete** | Dashboard, species page, HTMX partials for live updates |
| Static files | `routes/static_files.rs` | **Complete** | Embedded HTMX JS (air-gapped compatible) |

### birdnet-integrations

| Module | File | Status | Notes |
|--------|------|--------|-------|
| BirdWeather | `birdweather.rs` | **Complete** | HTTP client, retry with exponential backoff |

### birdnet-behavioral

| Module | File | Status | Notes |
|--------|------|--------|-------|
| Result types | `types.rs` | **Complete** | All analytics result/param types, residency classification |
| SQL builders | `queries.rs` | **Complete** | Sessionize, retention, funnel, patterns, next-species SQL |
| DuckDB connection | `connection.rs` | **Complete** | File-backed DuckDB, sync from SQLite, real-time insert, behavioral queries |

### Binary

| Module | File | Status | Notes |
|--------|------|--------|-------|
| Entry point | `src/main.rs` | **Complete** | CLI, config, DB recovery, detection daemon, WebSocket bridge, DuckDB analytics |

## Recent Changes (March 12, 2026)

### DuckDB Behavioral Analytics Integration

- File-backed DuckDB connection module (`birdnet-behavioral/connection.rs`)
- Incremental SQLite → DuckDB sync (offline-compatible, no sqlite_scanner extension)
- Real-time detection insertion alongside SQLite writes
- Behavioral query execution: `sessionize()`, `retention()`, `funnel()`, `next_species()`
- Optional `analytics` feature flag — default builds exclude DuckDB (~1min build vs ~7min)
- `--analytics-db` CLI flag and `BIRDNET_ANALYTICS_DB` env var
- DuckDB wired into AppState, event processor, and all analytics API routes

### HTMX Web Dashboard

- Server-rendered dashboard with dark theme (slate/sky palette)
- Dashboard page: stats grid, live detection table, top species sidebar
- Species page: full species list with detection counts and confidence
- HTMX partials for live updates:
  - `/pages/stats` — stats cards (60s polling)
  - `/pages/detections` — recent detections table (15s polling)
  - `/pages/top-species` — top species sidebar (60s polling)
  - `/pages/species-list` — full species table
  - `/pages/health-badge` — health status indicator (30s polling)
- Embedded HTMX JS (~50KB, air-gapped compatible)
- HTML templates compiled into binary via `include_str!`
- XSS prevention via HTML escaping
- Pure Rust date calculation (no chrono dependency)

### HTTP Status Code Improvements

- All API endpoints return proper HTTP status codes
- 500 Internal Server Error for database/task failures
- 503 Service Unavailable for degraded health and missing extensions
- 400 Bad Request for missing required query parameters
- Analytics status endpoint (`/analytics/status`) for capability introspection

### Analytics API Endpoints (DuckDB-backed)

- `GET /analytics/sessions` — Activity sessionization (gap-based grouping)
- `GET /analytics/retention` — Species return pattern analysis
- `GET /analytics/funnel` — Dawn chorus sequence validation
- `GET /analytics/next-species` — Next-species prediction
- `GET /analytics/status` — Capability and configuration status
- All endpoints cfg-gated: return "unavailable" without analytics feature

## Next Priority Items

1. **Apprise notifications** — Push alerts for rare species
2. **Flickr/Wikipedia** — Species image caching
3. **RTSP stream management** — Audio source handling
4. **Authentication** — Matching current Caddy basic auth setup
5. **Export endpoints** — CSV, JSON bulk export

## Test Coverage

| Crate | Unit Tests | Integration Tests | Status |
|-------|-----------|------------------|--------|
| birdnet-core | 54 (config, decode, resample, spectrogram, labels, model, pipeline, daemon) | 19 (audio pipeline + real Pica pica) | All passing |
| birdnet-db | 19 (sqlite, resilience, migration) | — | All passing |
| birdnet-web | 9 (websocket broadcast, pages, static files) | 16 (HTTP API + HTMX pages + analytics status) | All passing |
| birdnet-integrations | 2 (birdweather client) | — | All passing |
| birdnet-behavioral | 10 (types, queries) | — | All passing |
| **Total** | **94** | **35** | **129 tests passing** |

## Lines of Code (Rust, excluding tests)

| Crate | ~LOC | Notes |
|-------|------|-------|
| birdnet-core | ~1,200 | Audio pipeline + inference + daemon |
| birdnet-db | ~600 | Full implementation |
| birdnet-web | ~900 | REST API + WebSocket + HTMX pages + analytics |
| birdnet-integrations | ~150 | BirdWeather only |
| birdnet-behavioral | ~750 | Types + SQL builders + DuckDB connection |
| main.rs | ~430 | Entry point + daemon bridge + DuckDB wiring |
| Integration tests | ~900 | audio_pipeline.rs + web_api.rs |
| **Total** | **~4,930** | Production Rust code |

## Key Dependencies

| Purpose | Crate | Version | Pure Rust |
|---------|-------|---------|-----------|
| ONNX inference | `tract-onnx` | 0.22 | Yes |
| Audio decode | `symphonia` | 0.5.5 | Yes |
| Resampling | `rubato` | 1.0.1 | Yes |
| FFT | `realfft` | 3 | Yes |
| File watching | `notify` | 8 | Yes |
| Web framework | `axum` | 0.8.8 | Yes |
| SQLite | `rusqlite` | 0.38 | No (bundled C) |
| DuckDB | `duckdb` | 1.2 | No (bundled C++, optional) |

## Appendix: Python Component Map

### Entry Points

| File | Type | Systemd Service |
|------|------|-----------------|
| `scripts/birdnet_analysis.py` | Python daemon | `birdnet_analysis.service` |
| `scripts/birdnet_recording.sh` | Bash daemon | `birdnet_recording.service` |
| `scripts/web/main.py` | FastAPI server | `birdnet_web.service` |
| `scripts/disk_check.sh` | Cron job | crontab |

### Python LOC Being Replaced

| Component | LOC | Rust Status |
|-----------|-----|------------|
| Core Pipeline | ~2,500 | 95% replaced |
| Web Server | ~7,000+ | 85% replaced |
| Shell Scripts | ~500 | Not started |
| Tests | ~19,000 | Rust tests written alongside implementation |
| **Total** | **~29,000** | **~4,930 LOC Rust replaces ~9,500 LOC Python** |

---

[← Risks](12-risks.md) | [Back to Index](../RUST_ARCHITECTURE_PLAN.md)
