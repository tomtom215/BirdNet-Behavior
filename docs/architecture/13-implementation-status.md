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
| 5 | Web Server | **Complete** | 100% |
| 6 | Integrations | **Partial** | 80% |
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
| Export routes | `routes/export.rs` | **Complete** | CSV and JSON bulk export for detections and species, date range filtering |
| HTMX pages | `routes/pages.rs` | **Complete** | Dashboard, species, analytics pages, HTMX partials for live updates |
| Image routes | `routes/images.rs` | **Complete** | Species image metadata and file serving from Wikipedia cache |
| Auth middleware | `auth.rs` | **Complete** | HTTP Basic Auth, constant-time comparison, pure Rust base64 |
| Static files | `routes/static_files.rs` | **Complete** | Embedded HTMX JS (air-gapped compatible) |

### birdnet-integrations

| Module | File | Status | Notes |
|--------|------|--------|-------|
| BirdWeather | `birdweather.rs` | **Complete** | HTTP client, retry with exponential backoff, wired into event processor |
| Apprise | `apprise.rs` | **Complete** | Push notifications, per-species cooldown, confidence threshold, species watchlist, retry with backoff |
| Species images | `species_images.rs` | **Complete** | Wikipedia/Wikimedia image caching, on-disk cache, in-memory index |

### birdnet-behavioral

| Module | File | Status | Notes |
|--------|------|--------|-------|
| Result types | `types.rs` | **Complete** | All analytics result/param types, residency classification |
| SQL builders | `queries.rs` | **Complete** | Sessionize, retention, funnel, patterns, next-species SQL |
| DuckDB connection | `connection.rs` | **Complete** | File-backed DuckDB, sync from SQLite, real-time insert, behavioral queries |

### Binary

| Module | File | Status | Notes |
|--------|------|--------|-------|
| Entry point | `src/main.rs` | **Complete** | CLI, config, DB recovery, detection daemon, WebSocket bridge, DuckDB analytics, Apprise + BirdWeather, image cache, auth |

## Recent Changes (March 12, 2026)

### Species Image Caching (Wikipedia/Wikimedia)

- `ImageCache` in `birdnet-integrations/species_images.rs` (720+ lines)
- Fetches species thumbnails from Wikipedia `MediaWiki` API by scientific name
- On-disk cache with in-memory index for air-gapped operation
- `get_image()`, `download_image()`, `is_cached()`, `get_cached()` methods
- Background download/caching via `tokio::spawn`
- Pure Rust URL encoding (no external crate)
- 16 unit tests covering cache operations, URL encoding, Wikipedia response parsing

### Species Image API Endpoints

- `GET /api/v2/species/image/{scientific_name}` — Image metadata JSON (cache status, URL, description)
- `GET /api/v2/species/image/{scientific_name}/file` — Serve cached image bytes
- `--image-cache-dir` CLI flag for enabling image cache

### HTTP Basic Authentication

- `AuthConfig` with constant-time credential comparison
- Pure Rust base64 decoder (no external crate)
- Excluded paths for health checks and WebSocket endpoints
- `--auth` support via `CADDY_USER`/`CADDY_PWD` config compatibility
- 11 unit tests

### Audio Capture Manager

- `CaptureManager` struct for subprocess lifecycle management (arecord/ffmpeg)
- `start()`, `stop()`, `check_and_restart()`, `is_running()` methods
- Max restart count (10) with logging
- `--alsa-device`, `--rtsp-url`, `--segment-duration` CLI flags
- 3 unit tests

### Analytics HTMX Dashboard Page

- Analytics page (`/analytics`) with sessions, retention, next-species prediction cards
- 5 HTMX partials: status, sessions, retention, next-species, config
- Feature-gated: graceful "unavailable" message when analytics not compiled/configured
- All partials follow existing pattern (spawn_blocking, HTML response, XSS-safe)

### Species Detail Page

- Species detail page (`/species/detail?name=...`) with per-species analytics
- 4 HTMX partials: summary stats, hourly activity chart, recent detections, species info
- SVG bar charts for hourly activity and 7-day detection trends (server-rendered, no JS)
- Clickable species names throughout dashboard (detections table, species list, top species sidebar)
- New SQLite queries: `species_summary()`, `detections_by_species()`, `species_hourly_activity()`, `daily_counts()`
- Wikipedia species image integration in detail view (when `--image-cache-dir` enabled)
- Pure Rust percent-encoding (`simple_url_encode`) for URL-safe species name parameters
- 10 new tests (4 unit + 6 integration)

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

### Apprise Push Notifications

- Full Apprise client (`birdnet-integrations/apprise.rs`) with per-species cooldown
- `NotifyConfig`: min_confidence, species_watchlist, cooldown period
- `should_notify()` filter: confidence threshold, watchlist, cooldown deduplication
- `notify_detection()` / `send_notification()` with retry + exponential backoff
- Wired into detection event processor via `tokio::sync::Mutex` (async-safe)
- `--apprise-url` CLI flag and `BIRDNET_APPRISE_URL` env var
- Config file keys: `APPRISE_URL`, `APPRISE_MIN_CONFIDENCE`, `APPRISE_COOLDOWN`, `APPRISE_WATCHLIST`
- 9 unit tests covering all filtering logic

### BirdWeather Integration Wiring

- BirdWeather client wired into detection event processor
- Every detection posted to `app.birdweather.com` via async task
- `--birdweather-token`, `--latitude`, `--longitude` CLI flags
- `BIRDNET_BIRDWEATHER_TOKEN`, `BIRDNET_LATITUDE`, `BIRDNET_LONGITUDE` env vars
- Config file keys: `BIRDWEATHER_TOKEN`, `LATITUDE`, `LONGITUDE`
- ISO 8601 timestamp formatting for API compatibility

### Export Endpoints

- `GET /api/v2/detections/export` — Bulk detection export (CSV default, `?format=json`)
- `GET /api/v2/species/export` — Species summary export (CSV default, `?format=json`)
- Date range filtering: `?from=YYYY-MM-DD&to=YYYY-MM-DD`
- CSV format compatible with BirdNET-Pi `BirdDB.txt` column order
- RFC 4180 compliant CSV escaping
- `Content-Disposition` header for download prompts
- 7 unit tests + 5 integration tests

## Next Priority Items

1. **RTSP stream management** — Full audio source lifecycle (inspired by LyreBirdAudio)
2. **Spectrogram visualization** — Spectral chart endpoint for web dashboard
3. **Shell script replacement** — `disk_check.sh`, system management scripts

## Test Coverage

| Crate | Unit Tests | Integration Tests | Status |
|-------|-----------|------------------|--------|
| birdnet-core | 57 (config, decode, resample, spectrogram, labels, model, pipeline, daemon) | 19 (audio pipeline + real Pica pica) | All passing |
| birdnet-db | 32 (sqlite, resilience, migration) | — | All passing |
| birdnet-web | 36 (websocket, pages, static files, export CSV, auth, url encode) | 37 (HTTP API + HTMX pages + analytics + export + species detail) | All passing |
| birdnet-integrations | 27 (birdweather + apprise + species images) | — | All passing |
| birdnet-behavioral | 10 (types, queries) | — | All passing |
| **Total** | **162** | **56** | **208 tests passing** |

## Lines of Code (Rust, excluding tests)

| Crate | ~LOC | Notes |
|-------|------|-------|
| birdnet-core | ~1,200 | Audio pipeline + inference + daemon |
| birdnet-db | ~700 | Full implementation + species queries + confidence distribution |
| birdnet-web | ~1,900 | REST API + WebSocket + HTMX pages + analytics + export + auth + images + species detail |
| birdnet-integrations | ~1,100 | BirdWeather + Apprise + Wikipedia species images |
| birdnet-behavioral | ~750 | Types + SQL builders + DuckDB connection |
| main.rs | ~700 | Entry point + daemon bridge + DuckDB wiring + Apprise + image cache + auth |
| Integration tests | ~1,000 | audio_pipeline.rs + web_api.rs |
| **Total** | **~7,700** | Production Rust code |

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
| **Total** | **~29,000** | **~5,600 LOC Rust replaces ~9,500 LOC Python** |

---

[← Risks](12-risks.md) | [Back to Index](../RUST_ARCHITECTURE_PLAN.md)
