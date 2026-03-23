# Implementation Status

> Current state of the Rust implementation. Last updated: **2026-03-23**.

## Table of Contents

- [Phase Summary](#phase-summary)
- [Detailed Status by Crate](#detailed-status-by-crate)
  - [birdnet-core](#birdnet-core)
  - [birdnet-db](#birdnet-db)
  - [birdnet-web](#birdnet-web)
  - [birdnet-integrations](#birdnet-integrations)
  - [birdnet-migrate](#birdnet-migrate)
  - [birdnet-behavioral](#birdnet-behavioral)
  - [birdnet-timeseries](#birdnet-timeseries)
  - [Binary](#binary)
- [Recent Changes](#recent-changes)
- [Test Coverage](#test-coverage)
- [Lines of Code](#lines-of-code)
- [Key Dependencies](#key-dependencies)

---

## Phase Summary

| Phase | Description | Status | Completion |
|-------|-------------|--------|------------|
| 0 | Scaffolding | **Complete** | 100% |
| 1 | Data Layer | **Complete** | 100% |
| 2 | Audio Pipeline | **Complete** | 100% |
| 3 | ML Inference | **Complete** | 100% |
| 4 | Detection Daemon | **Complete** | 100% |
| 5 | Web Server + Dashboard | **Complete** | 100% |
| 6 | Integrations | **Complete** | 100% |
| 7 | Audio Capture | **Complete** | 100% |
| 8 | BirdNET-Pi Migration | **Complete** | 100% |
| 9 | Analytics Dashboards | **Complete** | 100% |
| 10 | Assembly + Polish | **Complete** | 100% |

---

## Detailed Status by Crate

### birdnet-core

| Module | File | Status | Notes |
|--------|------|--------|-------|
| Config parser | `config.rs` | **Complete** | INI parsing, PHP quote stripping, full tests |
| Audio decode | `audio/decode.rs` | **Complete** | symphonia WAV/FLAC/MP3, mono downmix |
| Audio resample | `audio/resample.rs` | **Complete** | rubato polynomial resampler, chunked processing |
| Mel spectrogram | `audio/spectrogram.rs` | **Complete** | Pure Rust realfft, librosa-compatible, 128 mel bands |
| Audio capture | `audio/capture/` | **Complete** | arecord + ffmpeg subprocess management, restart logic |
| Detection types | `detection/types.rs` | **Complete** | Detection struct, RecordingFile parser, serde |
| Detection pipeline | `detection/pipeline.rs` | **Complete** | File watching, chunking, spectrogram prep |
| Detection daemon | `detection/daemon.rs` | **Complete** | File watcher, inference loop, event broadcasting |
| Inference labels | `inference/labels.rs` | **Complete** | BirdNET label format parser, lookup by sci/common name |
| Inference model | `inference/model.rs` | **Complete** | tract-onnx ONNX inference, sigmoid/softmax, multi-model |
| Disk management | `audio/capture/disk/` | **Complete** | Disk usage, recording stats, auto-cleanup (split: mod, manager, purge) |
| Live spectrogram | `audio/spectrogram/live.rs` | **Complete** | inotify watcher, mel spectrogram push, WebSocket broadcast |
| tmpfs support | `audio/capture/tmpfs.rs` | **Complete** | Transient audio tmpfs mount/unmount, systemd unit generation |
| Audio extraction | `audio/extraction/` | **Complete** | Modular: config, format, extractor, convert, wav (6 sub-modules) |

### birdnet-db

| Module | File | Status | Notes |
|--------|------|--------|-------|
| SQLite CRUD | `sqlite/` | **Complete** | WAL mode, insert, detection queries, pagination, search |
| Heatmap queries | `sqlite/queries/heatmap.rs` | **Complete** | `weekly_heatmap`, `hourly_totals`, `species_daily_heatmap` |
| Correlation queries | `sqlite/queries/correlation.rs` | **Complete** | `top_cooccurrence_pairs`, `companion_species`, `temporal_cooccurrence` |
| Settings | `settings.rs` | **Complete** | SQLite-backed key/value, categories, bulk update |
| Notification log | `notifications.rs` | **Complete** | Per-channel log, stats, prune, status enum |
| Resilience | `resilience.rs` | **Complete** | Backup, restore, integrity check, auto-recovery |
| Migrations | `migration.rs` | **Complete** | 3 schema migrations, idempotent, version tracking |

### birdnet-web

| Module | File | Status | Notes |
|--------|------|--------|-------|
| Server setup | `server.rs` | **Complete** | axum, middleware, graceful shutdown |
| App state | `state.rs` | **Complete** | Arc<Mutex>, auto-migration, DuckDB, broadcast, log broadcaster, recording dir |
| Auth | `auth.rs` | **Complete** | HTTP Basic Auth, constant-time compare, pure Rust base64 |
| System info | `system_info.rs` | **Complete** | CPU/memory/temperature/uptime via sysinfo |
| Detection routes | `routes/detections.rs` | **Complete** | Recent, by-date, paginated, search |
| Species routes | `routes/species.rs` | **Complete** | Top species, hourly activity, detail, search |
| Analytics routes | `routes/analytics.rs` | **Complete** | Sessions, retention, funnel, next-species (DuckDB) |
| TimeSeries routes | `routes/timeseries.rs` | **Complete** | Activity, diversity, trend, peak, gap, sessions |
| Export routes | `routes/export.rs` | **Complete** | CSV + JSON, date range, BirdNET-Pi CSV compatible |
| WebSocket | `routes/websocket.rs` | **Complete** | Live detection streaming, broadcast, ping/pong |
| Recording routes | `routes/recordings.rs` | **Complete** | Audio file listing + secure streaming, path-traversal protection |
| Image routes | `routes/images.rs` | **Complete** | Species image metadata + file serving from Wikipedia cache |
| Static files | `routes/static_files.rs` | **Complete** | Embedded HTMX JS + SSE extension (air-gapped) |
| Dashboard page | `routes/pages/dashboard.rs` | **Complete** | Live detections with inline audio player, top species, stats |
| Species pages | `routes/pages/species_pages.rs` | **Complete** | List, search, detail with hourly chart + Wikipedia image |
| Heatmap page | `routes/pages/heatmap.rs` | **Complete** | SVG hour×day grid + hourly bar chart (HTMX partials) |
| Correlation page | `routes/pages/correlation.rs` | **Complete** | Co-occurrence pairs + companion species lookup |
| Analytics page | `routes/pages/behavioral.rs` | **Complete** | Sessions, retention, funnel, next-species (feature-gated) |
| Charts page | `routes/pages/charts.rs` | **Complete** | Daily, hourly, confidence distribution SVG charts |
| TimeSeries page | `routes/pages/timeseries_dash.rs` | **Complete** | Time-series analytics dashboard |
| Admin settings | `routes/admin/settings.rs` | **Complete** | Audio, location, detection, notifications, email, species, system |
| Admin migration | `routes/admin/migration.rs` | **Complete** | File upload + server-path import, validate, progress polling |
| Admin migration render | `routes/admin/migration/render.rs` | **Complete** | HTML rendering for all migration states |
| Admin system | `routes/admin/system.rs` | **Complete** | CPU/memory/temperature card, disk, recordings, backup trigger |
| Admin backups | `routes/admin/backup.rs` | **Complete** | List/download/delete backup files with path-traversal protection |
| Admin logs | `routes/admin/logs.rs` | **Complete** | SSE live log stream, ring buffer warm-up, full log viewer page |
| Admin notifications | `routes/admin/notifications.rs` | **Complete** | Notification history log + stats + prune |
| Admin update | `routes/admin/update.rs` | **Complete** | Update check + apply (GitHub Releases) |
| Admin species tester | `routes/admin/species_tester.rs` | **Complete** | Filter preview: include/exclude/SF thresh simulation |
| Spectrogram routes | `routes/spectrogram/` | **Complete** | Modular: render, font, png, colormap (split from monolith) |
| Spectrogram WS | `routes/spectrogram_ws.rs` | **Complete** | Live spectrogram WebSocket broadcast |
| Audio player page | `routes/pages/audio_player.rs` | **Complete** | Custom player with spectrogram, playhead, speed control |
| Metrics routes | `routes/health.rs` | **Complete** | Prometheus `/api/v2/metrics` endpoint, process stats |

### birdnet-integrations

| Module | File | Status | Notes |
|--------|------|--------|-------|
| Email | `email/` | **Complete** | SMTP via lettre + rustls, HTML + plain multipart, per-species cooldown, confidence threshold |
| Apprise | `apprise.rs` | **Complete** | 80+ notification channels, cooldown, watchlist, retry backoff |
| BirdWeather | `birdweather.rs` | **Complete** | Detection + soundscape uploads, retry with exponential backoff |
| Species images | `species_images/` | **Complete** | Wikipedia/Wikimedia cache, on-disk + in-memory index, background download |
| Auto-update | `auto_update.rs` | **Complete** | GitHub Releases version check, binary download + atomic replace |

### birdnet-migrate

| Module | File | Status | Notes |
|--------|------|--------|-------|
| Traits | `traits.rs` | **Complete** | `Migrator`, `Validator`, `SchemaDetector` traits |
| Error types | `error.rs` | **Complete** | `MigrateError` with `Source`, `Dest`, `Validation`, `Query` variants |
| Schema detection | `schema.rs` | **Complete** | Detects BirdNET-Pi SQLite and `BirdDB.txt` schemas |
| Progress | `progress.rs` | **Complete** | Thread-safe `ProgressHandle` with stage + row count |
| BirdNET-Pi validator | `birdnet_pi/validator.rs` | **Complete** | 6 required checks, 2 advisory checks, data quality report |
| BirdNET-Pi importer | `birdnet_pi/importer.rs` | **Complete** | Batch insert, duplicate skip, transaction-backed |
| BirdNET-Pi species report | `birdnet_pi/species_report.rs` | **Complete** | Pre-migration stats, post-migration per-species comparison |
| Public API | `birdnet_pi/mod.rs` | **Complete** | `validate_source`, `run_migration` convenience functions |

### birdnet-behavioral

| Module | File | Status | Notes |
|--------|------|--------|-------|
| Result types | `types.rs` | **Complete** | All analytics result/param types, residency classification |
| SQL builders | `queries.rs` | **Complete** | Sessionize, retention, funnel, patterns, next-species SQL |
| DuckDB connection | `connection.rs` | **Complete** | File-backed DuckDB, sync from SQLite, real-time insert, behavioral queries |

### birdnet-timeseries

| Module | File | Status | Notes |
|--------|------|--------|-------|
| Activity | `activity.rs` | **Complete** | Hourly/daily/weekly detection counts |
| Diversity | `diversity.rs` | **Complete** | Shannon index, species richness, per-hour diversity |
| Trend | `trend.rs` | **Complete** | Rolling window trends, moving averages |
| Peak | `peak.rs` | **Complete** | Peak activity detection, dawn/dusk windows |
| Gap | `gap.rs` | **Complete** | Silent period detection and characterization |
| Sessions | `sessions.rs` | **Complete** | Behavioral session windows |

### Binary

| Module | File | Status | Notes |
|--------|------|--------|-------|
| Entry point | `src/main.rs` + `src/helpers.rs` | **Complete** | CLI, DB recovery, daemon, server, all integrations wired |
| Detection daemon bridge | `src/daemon.rs` | **Complete** | Event processor: SQLite, DuckDB, WebSocket, Apprise, BirdWeather, Email |
| Audio capture | `src/capture.rs` | **Complete** | arecord + ffmpeg subprocess lifecycle management |
| Integrations | `src/integrations.rs` | **Complete** | Apprise, BirdWeather, Email, Auth client factories |
| CLI | `src/cli.rs` | **Complete** | All flags: model, labels, watch-dir, analytics, apprise, birdweather, auth, etc. |

---

## Recent Changes

### 2026-03-23

#### File Modularity Refactoring (Sprint 11)
- Split `settings/render.rs` (662 lines) → `settings/render/` module (mod, audio, location, detection, notifications, species, system, email — 7 sub-modules)
- Split `export.rs` (601 lines) → `export/` module (mod, csv, birddb, ebird — 4 sub-modules)
- Split `system_controls.rs` (600 lines) → `system_controls/` module (mod, data, backup, service, update — 5 sub-modules)
- Split `main.rs` (514 lines) → `main.rs` + `helpers.rs` (startup initialization, disk manager, Avahi mDNS)
- Refactored `state.rs` (550→~320 lines) — eliminated builder pattern duplication with `unwrap_inner`/`rebuild_inner` helpers
- All source files now under 600 lines (down from 8 files over 500)

#### Prometheus Metrics Endpoint
- `GET /api/v2/metrics` — Prometheus text exposition format
- Exports: `birdnet_info`, `birdnet_uptime_seconds`, `birdnet_detections_total`, `birdnet_species_total`, `birdnet_process_resident_memory_bytes`, `birdnet_cpu_count`, `birdnet_analytics_enabled`
- Compatible with Prometheus, Grafana Agent, Victoria Metrics scrapers

#### Enhanced Health Check
- `GET /api/v2/health` now includes `version`, `analytics` fields
- Returns 200 OK (healthy) or 503 Service Unavailable (degraded)

#### Bug Fixes
- Fixed pre-existing route conflict: duplicate `GET /admin/species/test` registration
- Fixed doctest failure in `tmpfs::generate_systemd_mount_unit`

### 2026-03-20

#### File Modularity Refactoring
- Split `spectrogram.rs` (621 lines) → `spectrogram/` module (mod, render, font, png, colormap)
- Split `extraction.rs` (753 lines) → `extraction/` module (mod, format, config, extractor, convert, wav)
- Split `disk.rs` (837 lines) → `disk/` module (mod, manager, purge)
- Split `web_api.rs` (1491 lines) → `web_api.rs` + `web_api_detections.rs`
- All source files now under 650 lines for maintainability

#### Live Spectrogram Daemon
- `birdnet-core::audio::spectrogram::live` — inotify file watcher + mel spectrogram computation
- `SpectrogramFrame` struct with normalized data for WebSocket transmission
- `birdnet-web::routes::spectrogram_ws` — WebSocket endpoint at `/api/v2/ws/spectrogram`
- `SpectrogramBroadcast` channel integrated into `AppState`

#### Binary Auto-Update
- `birdnet-integrations::auto_update` — check GitHub Releases for new versions
- Semver comparison (hand-rolled, no external crate)
- Atomic binary replace: download → temp file → chmod → rename
- Admin endpoints: `GET /admin/update/check`, `POST /admin/update/apply`

#### tmpfs Transient Audio Support
- `birdnet-core::audio::capture::tmpfs` — mount/unmount tmpfs for transient audio
- `is_tmpfs_mounted()` checks `/proc/mounts`
- `generate_systemd_mount_unit()` for persistent configuration
- Reduces SD card write wear on Raspberry Pi deployments

#### Species Filter Tester
- `GET /admin/species/test?include=...&exclude=...&sf_thresh=...`
- Returns JSON with filtered species count, sample list, and filter summary
- Preview filter changes before applying to production settings

#### Custom Audio Player
- `GET /player/{filename}` — standalone player page with spectrogram visualization
- Playhead canvas overlay on spectrogram image (synced to audio position)
- Playback speed control (0.5x–2x), volume slider, download button
- Dark theme styling consistent with dashboard

### 2026-03-13

#### Email Alert Integration
- `EmailNotifier` wired into detection event processor (`src/daemon.rs`)
- Email settings in admin settings UI (SMTP host, port, user, pass, from, to, from name, STARTTLS, confidence, cooldown)
- `create_email_notifier()` reads from SQLite settings table at startup
- Per-species cooldown tracking (shared with confidence threshold suppression)
- Zero SMTP calls when below confidence threshold or in active cooldown window

#### Backup Management UI
- `GET /admin/system/backups` — lists `.db` backup files with size and creation date
- `GET /admin/system/backups/{name}` — secure download with canonical path validation
- `DELETE /admin/system/backups/{name}` — HTMX-wired row deletion with confirm dialog
- "Manage Backups" link added to system admin page
- Pure Rust Unix timestamp → Gregorian date (no chrono dependency)

#### Inline Audio Player
- Detection dashboard rows now include `<audio controls>` element for WAV playback
- Audio served from `/api/v2/recordings/{filename}` (existing secure endpoint)
- Only rendered when `file_name` is present in the detection record

#### BirdNET-Pi Migration Improvements
- Post-migration `PostMigrationReport` with per-species count comparison
- Pre-migration `MigrationReport` with top-20 species, date range, data quality
- `validate_source` returns 3-tuple: `(DetectedSchema, ValidationReport, MigrationReport)`

#### Activity Heatmap + Correlation
- `GET /heatmap` — SVG hour×day grid with heat color scale, legend, day labels
- `GET /correlation` — species co-occurrence pairs + companion species lookup
- `GET /admin/system/logs/page` — SSE live log viewer (filter by level, pause, auto-scroll)
- `GET /admin/system/logs` — SSE stream with 200-line ring buffer warm-up

#### Species Image Caching
- `ImageCache` in `birdnet-integrations/species_images.rs`
- Fetches thumbnails from Wikipedia MediaWiki API by scientific name
- On-disk cache with in-memory index for air-gapped operation
- Background download via `tokio::spawn`

---

## Test Coverage

| Crate | Tests | Status |
|-------|-------|--------|
| birdnet-core | 19 (audio pipeline, inference, daemon) | All passing |
| birdnet-db | 69 (sqlite, resilience, heatmap, correlation, settings, notifications) | All passing |
| birdnet-web | 145 (pages, admin, backup, settings, export, auth, websocket) | All passing |
| birdnet-integrations | 49 (email types/templates/smtp/cooldown, apprise, birdweather, images) | All passing |
| birdnet-behavioral | 10 (types, queries) | All passing |
| birdnet-migrate | 33 (schema, validator, importer, species_report) | All passing |
| birdnet-timeseries | 24 (all analytics modules) | All passing |
| Integration tests | 74 (audio pipeline end-to-end, web API, HTMX pages) | All passing |
| **Total** | **~600** | **All passing** |

---

## Lines of Code

| Crate | ~LOC | Notes |
|-------|------|-------|
| birdnet-core | ~6,900 | Audio pipeline + inference + daemon + capture + disk + spectrogram + tmpfs |
| birdnet-db | ~3,800 | CRUD + heatmap + correlation + settings + notifications + resilience |
| birdnet-web | ~16,200 | REST API + WS + HTMX pages + admin + player + spectrogram + update |
| birdnet-integrations | ~3,500 | Email + Apprise + BirdWeather + species images + auto-update |
| birdnet-migrate | ~2,300 | Traits + schema + validator + importer + species_report |
| birdnet-behavioral | ~1,100 | Types + SQL builders + DuckDB connection |
| birdnet-timeseries | ~2,900 | All time-series analytics + windowing |
| birdnet-scheduler | ~900 | Solar calculations + window management |
| Binary (`src/`) | ~2,400 | main.rs + helpers.rs + daemon.rs + capture.rs + integrations.rs + cli.rs |
| **Total** | **~39,800** | Production Rust (including inline tests) |

---

## Key Dependencies

| Purpose | Crate | Version | Pure Rust |
|---------|-------|---------|-----------|
| Web framework | `axum` | 0.8.8 | Yes |
| Async runtime | `tokio` | 1.50 | Yes |
| ONNX inference | `tract-onnx` | 0.22 | Yes |
| Audio decode | `symphonia` | 0.5.5 | Yes |
| Resampling | `rubato` | 1.0.1 | Yes |
| FFT | `realfft` | 3 | Yes |
| File watching | `notify` | 8 | Yes |
| Email (SMTP) | `lettre` | 0.11 | Yes (rustls TLS) |
| System monitoring | `sysinfo` | 0.32 | Yes |
| SSE streaming | `tokio-stream` | 0.1 | Yes |
| File streaming | `tokio-util` | 0.7 | Yes |
| SQLite | `rusqlite` | 0.38 | No (bundled C) |
| DuckDB | `duckdb` | 1.2 | No (bundled C++, optional) |
| CLI | `clap` | 4.5 | Yes |
| Serialization | `serde` + `serde_json` | 1 | Yes |
| Logging | `tracing` | 0.1 | Yes |

---

## Appendix: Python Component Map

### Entry Points Being Replaced

| File | Type | Systemd Service | Rust Status |
|------|------|-----------------|-------------|
| `scripts/birdnet_analysis.py` | Python daemon | `birdnet_analysis.service` | ✅ Complete |
| `scripts/birdnet_recording.sh` | Bash daemon | `birdnet_recording.service` | ✅ Complete |
| `scripts/web/main.py` | FastAPI server | `birdnet_web.service` | ✅ Complete |
| `scripts/disk_check.sh` | Cron job | crontab | ✅ Integrated |

### Python LOC Replaced

| Component | Python LOC | Rust LOC | Ratio |
|-----------|-----------|---------|-------|
| Core Pipeline | ~2,500 | ~1,600 | 0.64× |
| Web Server | ~7,000 | ~3,200 | 0.46× |
| Shell Scripts | ~500 | ~200 | 0.40× |
| Tests | ~19,000 | ~3,000 | 0.16× |
| **Total** | **~29,000** | **~8,000** | **0.28×** |

Rust requires ~28% of the Python LOC while providing better performance, safety, and single-binary deployment.

---

[← Risks](12-risks.md) | [Back to Index](../RUST_ARCHITECTURE_PLAN.md)
