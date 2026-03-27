# Implementation Status

> Current state of the Rust implementation. Last updated: **2026-03-27 (Sprint 16)**.

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
| Audio extraction | `audio/extraction/` | **Complete** | Modular: config, format, extractor, convert, wav, metadata (7 sub-modules) |
| WAV metadata | `audio/extraction/metadata.rs` | **Complete** | RIFF INFO LIST chunk embedding (INAM/IART/IPRD/ICMT/ICRD/ISFT); pure Rust |
| Audio quality | `audio/quality/` | **Complete** | SNR estimation, spectral flatness, noise-floor tracking, rain/wind detection (4 sub-modules) |

### birdnet-db

| Module | File | Status | Notes |
|--------|------|--------|-------|
| SQLite CRUD | `sqlite/` | **Complete** | WAL mode, insert, detection queries, pagination, search |
| Heatmap queries | `sqlite/queries/heatmap.rs` | **Complete** | `weekly_heatmap`, `hourly_totals`, `species_daily_heatmap` |
| Correlation queries | `sqlite/queries/correlation.rs` | **Complete** | `top_cooccurrence_pairs`, `companion_species`, `temporal_cooccurrence` |
| Settings | `settings.rs` | **Complete** | SQLite-backed key/value, categories, bulk update |
| Notification log | `notifications.rs` | **Complete** | Per-channel log, stats, prune, status enum |
| Resilience | `resilience.rs` | **Complete** | Backup, restore, integrity check, auto-recovery |
| Migrations | `migration.rs` | **Complete** | 10 schema migrations (v10 adds quarantine table), idempotent, version tracking |
| Alert rules | `alert_rules.rs` | **Complete** | Conditional detection-triggered actions (webhook/log/suppress); glob matching; CRUD |
| Quarantine queries | `sqlite/queries/quarantine.rs` | **Complete** | `insert_quarantine`, `approve_quarantine` (atomic TX), `reject_quarantine`, `delete_quarantine`, `prune_quarantine`, `list_quarantine`, `quarantine_stats`, `quarantine_pending_count` |

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
| Rate limiter | `rate_limit.rs` | **Complete** | Per-IP token-bucket, `429 + Retry-After`, X-Forwarded-For, stale-entry pruning |
| Admin alert rules | `routes/admin/rules.rs` | **Complete** | Create/delete/toggle rules UI; HTMX live table; species glob + confidence + time window |
| Admin data quality | `routes/admin/quality.rs` | **Complete** | Confidence distribution, daily trend, hourly profile, low-confidence species ranking |
| Quality SQL queries | `sqlite/queries/analytics.rs` | **Complete** | `quality_summary`, `confidence_trend`, `detection_quality_by_hour`, `low_confidence_species` |
| WS new-species flag | `routes/websocket.rs` | **Complete** | `is_new_today` field on `WsDetectionEvent`; populated per detection |
| Quarantine page | `routes/pages/quarantine.rs` | **Complete** | Rare-bird review page: stats, paginated HTMX list, approve/reject/delete actions, pending badge |

### birdnet-integrations

| Module | File | Status | Notes |
|--------|------|--------|-------|
| Email | `email/` | **Complete** | SMTP via lettre + rustls, HTML + plain multipart, per-species cooldown, confidence threshold |
| Apprise | `apprise.rs` | **Complete** | 80+ notification channels, cooldown, watchlist, retry backoff |
| BirdWeather | `birdweather.rs` | **Complete** | Detection + soundscape uploads, retry with exponential backoff |
| Species images | `species_images/` | **Complete** | Wikipedia/Wikimedia cache, on-disk + in-memory index, background download |
| Auto-update | `auto_update.rs` | **Complete** | GitHub Releases version check, binary download + atomic replace |
| MQTT | `mqtt/` | **Complete** | Pure-Rust MQTT 3.1.1 over TCP; CONNECT/CONNACK/PUBLISH/DISCONNECT; QoS 0; retain flag; no external MQTT library |
| MQTT HA Discovery | `mqtt/discovery.rs` | **Complete** | Home Assistant auto-discovery: sensor, binary_sensor entities; `--mqtt-ha-discovery` flag |

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
| Phenology types | `phenology/types.rs` | **Complete** | `PhenologyRecord`, `MigrationWindow`, `WeeklyAbundance`, `PhenologyParams`, `AbundanceParams` |
| Phenology timing | `phenology/timing.rs` | **Complete** | Migration timing, migration-window percentiles (DuckDB), first detection, inter-annual trend (LAG window) |
| Phenology abundance | `phenology/abundance.rs` | **Complete** | Weekly abundance index (relative, 0–1), peak weeks, monthly totals, species richness, effort-corrected abundance |

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
| Detection daemon bridge | `src/daemon.rs` | **Complete** | Event processor: SQLite, DuckDB, WebSocket, Apprise, BirdWeather, Email, MQTT; per-species threshold failures quarantined instead of dropped |
| Audio capture | `src/capture.rs` | **Complete** | arecord + ffmpeg subprocess lifecycle management |
| Integrations | `src/integrations.rs` | **Complete** | Apprise, BirdWeather, Email, Auth, MQTT client factories |
| CLI | `src/cli.rs` | **Complete** | All flags: model, labels, watch-dir, analytics, apprise, birdweather, auth, MQTT, quality-filter, etc. |

---

## Recent Changes

### 2026-03-27 (Sprint 16)

#### Rare Bird Quarantine System

Implements the last planned novel feature (Sprint 6.2 from IMPLEMENTATION_PLAN.md) — a full
triage workflow for detections that are uncertain but too interesting to silently discard.

**Database layer** (`crates/birdnet-db/`):
- **Migration v10** — new `quarantine` table: `id`, `date`, `time`, `sci_name`, `com_name`,
  `confidence`, `sf_probability`, `reason` (`below_sf_thresh` | `low_confidence` | `manual`),
  `reviewed`, `approved`, `file_name`, `lat`, `lon`, `week`, `created_at`.
  Three indexes: `reviewed`, `date`, `sci_name`.
- **`QuarantineReason` enum** — `BelowSfThresh`, `LowConfidence`, `Manual`; stores as
  `&'static str`, parses from DB strings, exposes human-readable `label()`.
- **Full CRUD**: `insert_quarantine` (deduplicates via `INSERT OR IGNORE`),
  `approve_quarantine` (atomic `INSERT OR IGNORE INTO detections … SELECT … FROM quarantine` +
  `UPDATE reviewed/approved` in a single transaction), `reject_quarantine`, `delete_quarantine`,
  `prune_quarantine` (removes reviewed entries older than N days to prevent unbounded growth).
- **Read queries**: `list_quarantine` (paginated, filtered by `QuarantineFilter`),
  `get_quarantine`, `count_quarantine`, `quarantine_pending_count`, `quarantine_stats`
  (pending / approved / rejected / total).
- **14 unit tests** in `quarantine.rs` covering all operations including approve idempotency,
  dedup behaviour, prune, and `QuarantineReason` round-tripping.

**Detection daemon** (`src/daemon.rs`):
- Per-species threshold failures now quarantine instead of silently dropping the detection.
  The `continue` path now calls `birdnet_db::sqlite::insert_quarantine` with `reason = LowConfidence`
  before continuing, so no detection data is lost when users set strict per-species thresholds.

**Web layer** (`crates/birdnet-web/`):
- **`/quarantine`** page — full HTMX page with:
  - Stats bar (pending / approved / rejected / total counts via `quarantine-stats` partial)
  - Filter tabs (Pending / Approved / Rejected / All) via query parameter
  - Paginated table with species link, confidence badge, reason, date/time, status, and action buttons
  - Audio player for associated recording (if file_name present)
  - Action buttons: **Approve** (confirm dialog, copies to detections), **Reject**, **Delete**
  - Reviewed entries only offer Delete to clean up history
- **`/pages/quarantine-pending-count`** — tiny partial polled every 60 s from the nav badge
  (renders empty string when count = 0, coloured badge span when > 0)
- **Nav badge** — Quarantine link added to `layout.html`; badge auto-refreshes every 60 s to
  alert users when new entries arrive
- **`render_page` updated** — handles `{{nav_quarantine}}` placeholder

**Tests** — `tests/web_api_quarantine.rs` (14 integration tests):
- Page render, nav link, stats partial (empty + seeded), list partial (empty + pending + all filter),
  pending count badge (zero + non-zero), approve / reject / delete action handlers including
  DB-state verification after each action.

### 2026-03-23 (Sprint 13)

#### CI/CD Workflow

New `.github/workflows/ci.yml` — four-job pipeline that runs on every push to `master`, `main`, and `claude/**` branches, and on every pull request:

- **fmt** — `cargo fmt --check --all` (rustfmt, zero diff required)
- **clippy** — `cargo clippy --workspace --all-targets -- -D warnings` (pedantic + nursery, zero warnings)
- **test** — `cargo test --workspace` (all lib, unit, and integration tests)
- **docs** — `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` (zero broken doc links)

Cargo registry, git sources, and the `target/` directory are cached between runs using `actions/cache@v4` keyed on `Cargo.lock` hash, keeping CI runtime under 5 minutes for incremental builds.

#### Per-IP Token-Bucket Rate Limiter

New `birdnet-web::rate_limit` module — protects API and admin endpoints from overload without external crates:

- **Token-bucket algorithm** (`RateLimiter`) — `requests_per_second` (sustained rate) + `burst_capacity` (burst allowance); deterministic, no jitter
- **Per-IP state** — `Mutex<HashMap<IpAddr, Bucket>>` with periodic stale-entry pruning (entries idle for `2 × window_secs` are removed)
- **`X-Forwarded-For` support** — optional reverse-proxy header extraction via `trust_x_forwarded_for` config flag
- **HTTP 429 response** — returns `Retry-After` header (seconds until next token available); compliant with RFC 6585
- **axum middleware** — `RateLimitLayer` integrates as standard `tower::Layer`; reads client IP from `ConnectInfo<SocketAddr>` extension
- **27 unit tests** — bucket fill/drain, burst behavior, pruning, rate enforcement, header correctness

#### Home Assistant MQTT Auto-Discovery

New `birdnet-integrations::mqtt::discovery` module — publishes HA MQTT discovery messages at startup so no `configuration.yaml` edits are needed:

- **Entities registered**: last-detected species (sensor), detection confidence (sensor), station status (binary\_sensor), total detections today (sensor)
- **Discovery topic format**: `homeassistant/<component>/<unique_id>/config` (HA standard)
- **Device grouping**: all entities share one HA device entry (`BirdNet-Behavior station`) for clean UI
- **Retained publish**: discovery messages use RETAIN=true so HA recovers entity state on restart
- **Cleanup support**: publishing an empty payload removes the entity (call `publish_remove`)
- **`--mqtt-ha-discovery`** CLI flag — opt-in; requires `--mqtt-host` to be set
- **14 unit tests** — topic format, JSON structure, device fields, round-trip serialization

#### Clippy Zero-Warning Compliance

Full workspace clippy audit under `cargo clippy --workspace --all-targets -- -D warnings` (pedantic + nursery lint set):

- Fixed 40+ lint warnings across 18 source files
- Categories addressed: `cast_precision_loss`, `cast_sign_loss`, `cast_possible_truncation`, `similar_names`, `many_single_char_names`, `items_after_statements`, `significant_drop_tightening`, `field_reassign_with_default`, `single_char_pattern`, `map_unwrap_or`, `needless_pass_by_value`, `too_many_lines`
- All `#[allow(clippy::...)]` annotations carry justification comments
- Zero warnings remain — CI gate enforces this on every push

#### Documentation Link Audit

All broken `rustdoc` cross-reference links resolved:

- Private constants changed from `` [`CONST`] `` to plain `` `CONST` `` (private items cannot be linked)
- Non-existent method references removed (`DailySchedule::for_today`)
- Feature-gated module references changed to plain backtick
- `[`load`]` → `[`Self::load`]` where explicit disambiguation required
- `[`RecordingWindow`]` → `[`crate::RecordingWindow`]` for cross-module links
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` passes with zero warnings

### 2026-03-23 (Sprint 12)

#### Audio Quality Pre-Filtering

New `birdnet-core::audio::quality` module — full four-stage quality pipeline:

- **SNR estimation** (`snr.rs`) — frame-based peak-to-noise-floor ratio with dBFS noise floor tracking
- **Spectral flatness** (`snr.rs`) — Wiener entropy (geometric/arithmetic mean ratio) to distinguish tonal signals from broadband noise
- **Noise floor tracking** (`noise_floor.rs`) — adaptive minimum-statistics estimator with 64-frame circular buffer, overestimation-corrected output
- **Rain/wind detection** (`rain_detector.rs`) — purely time-domain IIR high-pass (4 kHz) and low-pass (500 Hz) filters; DC-offset removal before spectral analysis prevents false positives from constant-amplitude signals; `MIN_RMS_FOR_ANALYSIS = 1e-4` gate for near-silent inputs
- **Composite score** (`mod.rs`) — weighted combination (SNR 40%, inverse flatness 40%, rain penalty 20%); `assess_quality()` returns `QualityScore` or `QualityError`

New CLI flags: `--quality-filter` (enable), `--quality-min-snr-db` (default 3.0 dB).

#### MQTT 3.1.1 Integration (Sprint 12)

New `birdnet-integrations::mqtt` module — pure-Rust MQTT publisher with no external library:

- **Wire-protocol** (`publisher.rs`) — CONNECT (username/password), CONNACK parsing, PUBLISH (QoS 0), DISCONNECT over raw `TcpStream`
- **Types** (`types.rs`) — `MqttConfig`, `QosLevel`, `DetectionPayload`, `MqttError`, `ConnAckError`
- **Client** (`mod.rs`) — `MqttClient::publish_detection()` and `publish_status()`; integrated into detection event processor
- Detections published to `{prefix}/detection/{species_name}` as JSON
- Optional RETAIN flag for Home Assistant sensor persistence
- Compatible with Mosquitto, Home Assistant MQTT integration, Node-RED, and any MQTT 3.1.1 broker

New CLI flags: `--mqtt-host`, `--mqtt-port` (1883), `--mqtt-client-id`, `--mqtt-username`, `--mqtt-password`, `--mqtt-topic-prefix` ("birdnet"), `--mqtt-retain`.

#### Phenology Analytics (Sprint 12)

New `birdnet-behavioral::phenology` module — migration timing and abundance analytics:

- **Timing** (`timing.rs`) — `phenology_timing_sql`: per-species first/last detection, peak week, detection count, year range (SQLite-compatible); `migration_window_sql`: arrival/departure percentiles via `percentile_cont` (DuckDB); `interannual_trend_sql`: year-over-year count change using `LAG` window function (DuckDB)
- **Abundance** (`abundance.rs`) — `weekly_abundance_sql`: normalized abundance index [0.0, 1.0] relative to peak week; `peak_weeks_sql`: top-N peak weeks per species; `monthly_totals_sql`; `weekly_richness_sql`; `effort_corrected_abundance_sql` (DuckDB, joins `recordings` table)
- SQL injection protection: species names are single-quote escaped throughout

#### Criterion Benchmarks (Sprint 12)

- `crates/birdnet-core/benches/audio_pipeline.rs` — `bench_mel_spectrogram`, `bench_audio_quality`, `bench_snr_estimation`, `bench_rain_detection`, `bench_noise_floor_tracker`; synthetic bird-call generator (harmonic + AM envelope) and deterministic LCG white noise
- `crates/birdnet-db/benches/db_queries.rs` — single insert, batch transactions (10/100/1000 rows), top-species query, recent detections, weekly heatmap, species LIKE search; all run against in-memory SQLite with production schema
- `criterion = { version = "0.5", features = ["html_reports"] }` added to workspace dev-dependencies

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
| birdnet-core | 27 (audio pipeline, inference, daemon, quality: SNR/flatness/noise-floor/rain/wind) | All passing |
| birdnet-db | 83 (sqlite, resilience, heatmap, correlation, settings, notifications, quarantine CRUD) | All passing |
| birdnet-web | 172 (pages, admin, backup, settings, export, auth, websocket, rate-limiter) | All passing |
| birdnet-integrations | 71 (email, apprise, birdweather, images, MQTT wire-encoding, offline-broker, HA discovery) | All passing |
| birdnet-behavioral | 18 (types, queries, phenology timing/abundance SQL correctness) | All passing |
| birdnet-migrate | 33 (schema, validator, importer, species_report) | All passing |
| birdnet-timeseries | 24 (all analytics modules) | All passing |
| Integration tests | 88 (audio pipeline end-to-end, web API, HTMX pages, quarantine routes) | All passing |
| **Total** | **~516** | **All passing** |

---

## Lines of Code

| Crate | ~LOC | Notes |
|-------|------|-------|
| birdnet-core | ~7,650 | Audio pipeline + inference + daemon + capture + disk + spectrogram + tmpfs + quality |
| birdnet-db | ~3,800 | CRUD + heatmap + correlation + settings + notifications + resilience |
| birdnet-web | ~16,600 | REST API + WS + HTMX pages + admin + player + spectrogram + update + rate-limiter |
| birdnet-integrations | ~4,350 | Email + Apprise + BirdWeather + species images + auto-update + MQTT + HA discovery |
| birdnet-migrate | ~2,300 | Traits + schema + validator + importer + species_report |
| birdnet-behavioral | ~1,650 | Types + SQL builders + DuckDB connection + phenology (timing + abundance) |
| birdnet-timeseries | ~2,900 | All time-series analytics + windowing |
| birdnet-scheduler | ~900 | Solar calculations + window management |
| Binary (`src/`) | ~2,500 | main.rs + helpers.rs + daemon.rs + capture.rs + integrations.rs + cli.rs |
| Benchmarks | ~350 | Criterion audio pipeline + DB query benchmarks |
| **Total** | **~42,650** | Production Rust (including inline tests and benchmarks) |

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
