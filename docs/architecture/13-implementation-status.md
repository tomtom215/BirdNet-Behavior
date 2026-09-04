# Implementation Status

> Current state of the BirdNet-Behavior implementation, crate by crate.

## Table of Contents

- [Detailed Status by Crate](#detailed-status-by-crate)
  - [birdnet-core](#birdnet-core)
  - [birdnet-db](#birdnet-db)
  - [birdnet-web](#birdnet-web)
  - [birdnet-integrations](#birdnet-integrations)
  - [birdnet-migrate](#birdnet-migrate)
  - [birdnet-behavioral](#birdnet-behavioral)
  - [birdnet-timeseries](#birdnet-timeseries)
  - [birdnet-scheduler](#birdnet-scheduler)
  - [Binary](#binary)
- [Test Coverage](#test-coverage)
- [Lines of Code](#lines-of-code)
- [Key Dependencies](#key-dependencies)

---

## Detailed Status by Crate

### birdnet-core

| Module | Location | Notes |
|--------|----------|-------|
| Config parser | `config.rs` + `config/` | INI parsing with PHP-style quote stripping; `config/validate.rs` holds the validation rules |
| i18n | `i18n.rs` | 36-language species-name lookup |
| Audio decode | `audio/decode.rs` | symphonia-based WAV / FLAC / MP3 decoder with mono downmix |
| Audio resample | `audio/resample.rs` | rubato polynomial resampler with chunked processing |
| Mel spectrogram | `audio/spectrogram/compute.rs` | Pure Rust realfft implementation, librosa-compatible |
| Live spectrogram | `audio/spectrogram/live.rs` | inotify watcher producing `SpectrogramFrame` broadcasts |
| Audio capture | `audio/capture/` | `arecord` / `ffmpeg` subprocess management with restart logic (PulseAudio and PipeWire go through `ffmpeg -f pulse`) |
| Disk management | `audio/capture/disk/` | Per-species retention, auto-purge, disk usage tracking |
| tmpfs support | `audio/capture/tmpfs.rs` | Transient audio mount, systemd unit generation |
| Audio extraction | `audio/extraction/` | Per-detection WAV extraction, format conversion, RIFF INFO metadata |
| Audio quality | `audio/quality/` | SNR, spectral flatness, noise-floor tracking, rain / wind detection |
| Detection types | `detection/types.rs` | `Detection` struct, `RecordingFile` parser, serde support |
| Detection pipeline | `detection/pipeline.rs` | Chunking, overlap, spectrogram preparation |
| Detection daemon | `detection/daemon.rs` | File-watcher event loop, inference dispatch, event broadcast |
| Privacy filter | `detection/privacy.rs` | Human-voice suppression with adjacent-chunk masking |
| Inference labels | `inference/labels.rs` | BirdNET label format parser, scientific / common name lookup |
| Inference model | `inference/model.rs` | ort session wrapper, sigmoid / softmax post-processing |
| Species filter | `inference/species_filter.rs` | Species occurrence metadata model and include / exclude lists |

### birdnet-db

| Module | Location | Notes |
|--------|----------|-------|
| Connection | `sqlite/connection.rs` | WAL mode, `Arc<Mutex<Connection>>`, PRAGMA tuning |
| Types | `sqlite/types.rs` | Detection row types, query result types |
| Query API | `sqlite/queries/` | Detections, species, analytics, correlation, heatmap, images, quarantine |
| Migrations | `migration.rs` | 41 idempotent schema migrations (v1…v41) with version tracking |
| Settings | `settings.rs` | SQLite-backed key-value store with categories |
| Resilience | `resilience.rs` | Backup, restore, integrity check, auto-recovery |
| Alert rules | `alert_rules.rs` | Detection-triggered actions (webhook / log / suppress), glob matching |
| Notifications | `notifications.rs` | Per-channel log, stats, prune |

### birdnet-web

| Module | Location | Notes |
|--------|----------|-------|
| Server setup | `server.rs` | axum router, middleware, graceful shutdown |
| Application state | `state.rs` | Shared state, auto-migration, broadcast channels |
| Auth | `auth_middleware.rs` + `session.rs` | Cookie-session sign-in over argon2id password hashing; `api_token.rs` holds the bearer gate for the write API |
| Rate limit | `rate_limit.rs` | Per-IP token-bucket, `429 + Retry-After`, stale-entry pruning |
| System info | `system_info.rs` | CPU / memory / temperature / uptime via sysinfo |
| Detection routes | `routes/detections.rs` | Recent, by-date, paginated, search |
| Species routes | `routes/species.rs` | Top species, hourly activity, detail, search |
| Analytics routes | `routes/analytics.rs` | Sessions, retention, funnel, next-species (DuckDB) |
| Time-series routes | `routes/timeseries.rs` | Activity, diversity, trend, peak, gap, sessions |
| Export routes | `routes/export/` | CSV, BirdDB.txt, and eBird CSV export |
| WebSocket | `routes/websocket.rs` | Live detection streaming with broadcast and ping / pong |
| Spectrogram WS | `routes/spectrogram_ws.rs` | Live mel spectrogram WebSocket stream |
| Recording routes | `routes/recordings.rs` | Audio listing and secure streaming with path-traversal protection |
| Image routes | `routes/images.rs` | Species image metadata and file serving |
| Static files | `routes/static_files.rs` | Embedded HTMX JS and SSE extension |
| Health | `routes/health.rs` | `/api/v2/health` JSON and `/api/v2/metrics` Prometheus exposition |
| HTMX pages | `routes/pages/` | Dashboard, species, gallery, life list, heatmap, correlation, behavioral, charts, time-series, quarantine, recordings, today, history, weekly report, audio player, kiosk, livestream, system health, notification center |
| Admin panel | `routes/admin/` | Settings, species thresholds, species tester, migration, system, backup, logs, notifications, update, alert rules, data quality |

### birdnet-integrations

| Module | Location | Notes |
|--------|----------|-------|
| Email | `email/` | SMTP via lettre + rustls, HTML + plain multipart, per-species cooldown |
| Apprise | `apprise.rs` | 80+ notification channels, cooldown, watchlist, retry backoff |
| BirdWeather | `birdweather.rs` | Detection and soundscape uploads with retry and exponential backoff |
| Species images | `species_images/` | Wikipedia / Wikimedia cache with on-disk + in-memory index |
| Auto-update | `auto_update/` | GitHub Releases version check, binary download, atomic replace |
| MQTT | `mqtt/` | Pure-Rust MQTT 3.1.1 client over TCP; CONNECT / PUBLISH / DISCONNECT; QoS 0 |
| HA Discovery | `mqtt/discovery.rs` | Home Assistant auto-discovery sensors and binary sensors |
| Heartbeat | `heartbeat.rs` | Outbound GET ping after each processed detection |
| Notification templates | `notification.rs` | `$variable` substitution for title / body templates |
| Weekly report | `weekly_report.rs` | Scheduled weekly report generator |

### birdnet-migrate

| Module | Location | Notes |
|--------|----------|-------|
| Traits | `traits.rs` | `Migrator`, `Validator`, `SchemaDetector` traits |
| Error types | `error.rs` | `MigrateError` with `Source`, `Dest`, `Validation`, `Query` variants |
| Schema detection | `schema.rs` | Detects BirdNET-Pi SQLite and `BirdDB.txt` schemas |
| Progress | `progress.rs` | Thread-safe `ProgressHandle` with stage and row counts |
| Validator | `birdnet_pi/validator.rs` | Required and advisory integrity checks, data quality report |
| Importer | `birdnet_pi/importer.rs` | Batch transactional insert with duplicate skip |
| CSV importer | `birdnet_pi/csv_importer.rs` | `BirdDB.txt` import path |
| Species report | `birdnet_pi/species_report.rs` | Pre- and post-migration per-species comparison |

### birdnet-behavioral

| Module | Location | Notes |
|--------|----------|-------|
| Types | `types.rs` | Result and parameter types, residency classification |
| Queries | `queries.rs` | Sessionize, retention, funnel, next-species SQL builders |
| Connection | `connection/` | File-backed DuckDB, sync from SQLite, query execution |
| Phenology timing | `phenology/timing.rs` | Migration timing percentiles, first detection, inter-annual trend |
| Phenology abundance | `phenology/abundance.rs` | Weekly abundance index, peak weeks, monthly totals, species richness |

### birdnet-timeseries

| Module | Location | Notes |
|--------|----------|-------|
| Activity | `queries/activity.rs`, `executor/activity.rs` | Hourly / daily / weekly detection counts |
| Diversity | `queries/diversity.rs`, `executor/diversity.rs` | Shannon index, species richness, per-hour diversity |
| Trend | `queries/trend.rs`, `executor/trend.rs` | Rolling window trends and moving averages |
| Peak | `queries/peak.rs`, `executor/peak.rs` | Peak activity detection, dawn / dusk windows |
| Gap | `queries/gap.rs` | Silent-period detection and characterisation |
| Sessions | `executor/sessions.rs` | Behavioural session windows |
| Windows | `window/` | Tumbling, sliding, hopping, and session windowing primitives |

### birdnet-scheduler

| Module | Location | Notes |
|--------|----------|-------|
| Solar calculation | `solar.rs` | NOAA / Meeus sunrise / sunset computation |
| Schedule | `schedule.rs` | All-day, solar, and fixed-window recording schedules |
| Window management | `window.rs` | Active recording window representation |
| Night inhibit | `inhibit.rs` | Suppress recording during configured night hours |
| Traits | `traits.rs` | `Scheduler` trait for pluggable schedule sources |

### Binary

| Module | Location | Notes |
|--------|----------|-------|
| Entry point | `src/main.rs` + `src/helpers.rs` | CLI parse, DB recovery, daemon, web server, integration wiring |
| Application wiring | `src/app.rs` | Flag / env / config / settings precedence resolution and startup wiring |
| Detection bridge | `src/daemon/` | Event processor (`daemon/processor.rs`) for SQLite, DuckDB, WebSocket, Apprise, BirdWeather, email, MQTT; plus config resolution, daylight gating and disposition |
| Audio capture | `src/capture/` + `src/capture.rs` | arecord / ffmpeg subprocess lifecycle management |
| Integrations factory | `src/integrations/` + `src/integrations.rs` | Apprise, BirdWeather, email, MQTT client construction |
| CLI | `src/cli.rs` | clap argument definitions |
| Maintenance loop | `src/maintenance.rs` | Weekly VACUUM, backup pruning, clip and audit retention |
| Doctor | `src/doctor/` + `src/doctor.rs` | `--doctor` / `--doctor-json` preflight checks |
| Watchdog notifications | `src/sd_notify.rs` | `READY=1` / `WATCHDOG=1` / `STOPPING=1` for `Type=notify` |
| Log capture | `src/log_capture.rs` + `src/log_filter.rs` | Broadcast of `tracing` events to the admin log viewer |
| Channel report | `src/channel_report.rs` | `--channel-report` capture diagnostics |
| Support bundle | `src/support.rs` | Diagnostic bundle generation |
| Weekly report | `src/weekly_report.rs` | Weekly report runner |

---

## Test Coverage

Counted as `#[test]` / `#[tokio::test]` attributes:

```bash
grep -rn --include='*.rs' -E '^\s*#\[(test|tokio::test)' src crates tests | wc -l
```

| Crate | Test count | Coverage |
|-------|-----------:|----------|
| birdnet-core | 708 | Audio pipeline, inference, daemon, quality (SNR / flatness / noise floor / rain) |
| birdnet-db | 411 | SQLite, resilience, heatmap, correlation, settings, notifications, quarantine CRUD |
| birdnet-web | 819 | Pages, admin, backup, settings, export, auth, WebSocket, rate limiter |
| birdnet-integrations | 352 | Email, Apprise, BirdWeather, images, MQTT wire encoding, HA discovery |
| birdnet-behavioral | 105 | Types, query builders, phenology timing and abundance SQL correctness |
| birdnet-migrate | 72 | Schema, validator, importer, species report |
| birdnet-timeseries | 40 | All analytics modules |
| birdnet-scheduler | 27 | Solar calculation and recording-window scheduling |
| Binary (`src/`) | 799 | CLI, app wiring, daemon, doctor, maintenance, capture |
| Integration tests (`tests/`) | 290 | Audio pipeline end-to-end, web API, HTMX pages, quarantine routes |
| **Total** | **3 623** | |

Re-derive rather than trusting the figure above; it moves with every
branch.

---

## Lines of Code

Counted with:

```bash
find src crates -name '*.rs' | xargs cat | wc -l
```

| Crate | Approx. LOC |
|-------|------------:|
| birdnet-core | 29 790 |
| birdnet-db | 23 574 |
| birdnet-web | 63 545 |
| birdnet-integrations | 17 957 |
| birdnet-migrate | 4 648 |
| birdnet-behavioral | 6 758 |
| birdnet-timeseries | 3 561 |
| birdnet-scheduler | 1 246 |
| Binary (`src/`) | 33 425 |
| Benchmarks | 484 |
| **Total** | **184 504** |

Lines are counted with comments and inline tests included. Like the test
count, re-derive it rather than trusting the figure here.

---

## Key Dependencies

| Purpose | Crate | Version | Pure Rust |
|---------|-------|---------|-----------|
| Web framework | `axum` | 0.8 | Yes |
| Async runtime | `tokio` | 1.52 (lock 1.53) | Yes |
| ONNX inference | `ort` | 2.0.0-rc | No (C++ core, statically linked) |
| Audio decode | `symphonia` | 0.6 | Yes |
| Resampling | `rubato` | 4.0 | Yes |
| FFT | `realfft` | 3 | Yes |
| File watching | `notify` | 8 | Yes |
| Email (SMTP) | `lettre` | 0.11 | Yes (rustls TLS) |
| System monitoring | `sysinfo` | 0.39 | Yes |
| SSE streaming | `tokio-stream` | 0.1 | Yes |
| File streaming | `tokio-util` | 0.7 | Yes |
| SQLite | `rusqlite` | 0.40 | No (bundled C) |
| DuckDB | `duckdb` | 1.10505 (DuckDB 1.5.5) | No (bundled C++, optional) |
| CLI | `clap` | 4.6 | Yes |
| Serialization | `serde` + `serde_json` | 1 | Yes |
| Logging | `tracing` | 0.1 | Yes |

---

[← Risks](12-risks.md) | [Back to Index](../RUST_ARCHITECTURE_PLAN.md)
