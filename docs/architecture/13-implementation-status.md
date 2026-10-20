# Implementation Status

> What each crate contains, module by module.
>
> **Read this as a map, not a scoreboard.** The module tables were checked
> against the tree at one commit and are kept by hand. Every *count* that used
> to be printed here (migrations, tests, lines of code, doctor check families,
> dependency versions) went stale within a release, so each is now the command
> that prints it, with the value it printed on 2026-10-20 for reference. Run the
> command; do not quote the number.

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
| Audio resample | `audio/resample.rs` | rubato windowed-sinc resampler with chunked processing |
| Mel spectrogram | `audio/spectrogram/compute.rs` | Pure Rust realfft implementation, librosa-compatible |
| Live spectrogram | `audio/spectrogram/live.rs` | inotify watcher producing `SpectrogramFrame` broadcasts |
| Audio capture | `audio/capture/` | `arecord` / `ffmpeg` subprocess management with restart logic (PulseAudio and PipeWire go through `ffmpeg -f pulse`) |
| Disk management | `audio/capture/disk/` | Per-species retention, auto-purge, disk usage tracking |
| tmpfs support | `audio/capture/tmpfs.rs` | Transient audio mount, systemd unit generation |
| Audio extraction | `audio/extraction/` | Per-detection WAV extraction, format conversion, RIFF INFO metadata |
| Audio quality | `audio/quality/` | SNR, spectral flatness, noise-floor tracking, rain / wind detection |
| Detection types | `detection/types.rs` | `Detection` struct, `RecordingFile` parser, serde support |
| Detection pipeline | `detection/pipeline.rs` | Chunking, overlap, spectrogram preparation |
| Detection daemon | `detection/daemon/` (`mod.rs`, `run.rs`, `process.rs`) | File-watcher event loop, inference dispatch, event broadcast |
| Privacy filter | `detection/privacy.rs` | Human-voice suppression with adjacent-chunk masking |
| Inference labels | `inference/labels.rs` | BirdNET label format parser, scientific / common name lookup |
| Inference model | `inference/model.rs` | ort session wrapper; `compute_confidence` applies sigmoid(sensitivity × logit) to logit models (V2.4) and passes probability-output models (V3.0 preview) through unchanged |
| Species filter | `inference/species_filter.rs` | Species occurrence metadata model and include / exclude lists |

### birdnet-db

| Module | Location | Notes |
|--------|----------|-------|
| Connection | `sqlite/connection.rs` | `open_connection` / `open_or_create` / `open_readonly` / `quick_check`; WAL mode and PRAGMA tuning. The crate hands back plain `Connection`s — the single-writer `Mutex<Connection>` plus read-only `ReaderPool` live in `birdnet-web` (`state.rs`, `db_pool.rs`) |
| Types | `sqlite/types.rs` | Detection row types, query result types |
| Query API | `sqlite/queries/` | Detections, species, analytics, correlation, heatmap, images, quarantine, detection reviews, effort, imports, maintenance |
| Migrations | `migration.rs` | Idempotent, forward-only schema migrations numbered from 1, with version tracking (count: `awk '/pub const MIGRATIONS/,/^\];/' crates/birdnet-db/src/migration.rs \| grep -cE '^\s+version: [0-9]+,'`, which printed 54 on 2026-10-20); v42 re-keys `species_summary` by `(Com_Name, Sci_Name, hour, is_import)` |
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
| Health | `routes/health.rs` + `routes/system.rs` | `/api/v2/metrics` Prometheus exposition (`health.rs`); `/api/v2/health` JSON, `/api/v2/stats`, `/api/v2/system/disk` (`system.rs`) |
| HTMX pages | `routes/pages/` | Homes (`homes/`: patterns, reports, station) and Today, species pages, life list, heatmap, correlation, behavioral, charts, time-series, quarantine, recordings, history, weekly report, year in review, audio player, kiosk (`dashboard/kiosk.rs`), station health, notification center, detection detail, detection reviews, dawn chorus, migration, onboarding, search, command palette, provenance, changelog, help, `viz/`. `/gallery` is a redirect to `/species?view=photos` (`routes/redirects.rs`); the live stream lives in `routes/livestream.rs` |
| Admin panel | `routes/admin/` | Settings, species thresholds, species tester (`/admin/species/test`), migration, system, system controls, backup, backup recovery, logs, notifications, notification test, update, alert rules, data quality, accounts, audio sources, EQ curve, images, overview, doctor (`/admin/doctor`, `/admin/doctor.json`, `/admin/support-bundle`) |

### birdnet-integrations

| Module | Location | Notes |
|--------|----------|-------|
| Email | `email/` | SMTP via lettre + rustls, HTML + plain multipart, per-species cooldown |
| Apprise | `apprise.rs` | 80+ notification channels, cooldown, watchlist, retry backoff |
| BirdWeather | `birdweather.rs` | Detection and soundscape uploads with retry and exponential backoff |
| Species images | `species_images/` | Provider chain (`chain/`, `provider.rs`) over Wikipedia / Wikimedia and Flickr (`flickr/`) with on-disk + in-memory cache |
| Auto-update | `auto_update/` | GitHub Releases version check, binary download, atomic replace |
| MQTT | `mqtt/` | Pure-Rust MQTT 3.1.1 publisher over TCP or rustls TLS (`TlsConfig`); CONNECT / PUBLISH / DISCONNECT; detections at the configured `MqttConfig::qos` (default 0), presence and last-will retained at QoS 1 |
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
| Doctor | `src/doctor/` + `src/doctor.rs` | `--doctor` / `--doctor-json` preflight checks (one row of `CHECK_FAMILIES` per family; `grep -n 'const CHECK_FAMILIES' src/doctor.rs` shows the length, 26 on 2026-10-20), opt-in `--fix` repairs (`doctor/fix.rs`); also served at `/admin/doctor` |
| Watchdog notifications | `src/sd_notify.rs` | `READY=1` / `WATCHDOG=1` / `STOPPING=1` for `Type=notify` |
| Log capture | `src/log_capture.rs` + `src/log_filter.rs` | Broadcast of `tracing` events to the admin log viewer |
| Channel report | `src/channel_report.rs` | `--channel-report` capture diagnostics |
| Support bundle | `src/support.rs` | Diagnostic bundle generation |
| Weekly report | `src/weekly_report.rs` | Weekly report runner |

### Modules the tables above do not list

The tables name the modules that carry the architecture; every entry was
checked against `ls` at commit `dd10fe7`. These modules also exist and are
not listed above:

- **birdnet-core**: `audio/biquad.rs`, `audio/eq/`, `audio/soundlevel/`,
  `audio/quality/stream_fault.rs`, `civil.rs`, `season.rs`, `file_settle.rs`,
  `config/locale.rs`, `config/redact.rs`, `detection/corroboration.rs`,
  `detection/nocturnal.rs`, `detection/noise.rs`, `detection/dynamic_threshold/`
- **birdnet-db**: `accounts/`, `audio_levels.rs`, `audio_sources.rs`, `clock.rs`,
  `dynamic_thresholds.rs`, `outbound_queue.rs`, `phantoms.rs`, `sound_levels.rs`,
  `species_tracking.rs`, `thresholds.rs`, `weather.rs`
- **birdnet-web**: `analytics_cache.rs`, `api_token.rs`, `audit.rs`, `base_path/`,
  `client_ip.rs`, `db_pool.rs`, `diagnostics.rs`, `metrics.rs`, `notifier.rs`,
  `security.rs`, `session.rs`, `tls.rs`, `tracking.rs`, `urls.rs`;
  `routes/{api_write,auth_pages,feeds,livestream,openapi,redirects,share}.rs`
- **birdnet-integrations**: `dispatch/`, `offsite/` (`envelope`, `s3`, `sftp`,
  `sigv4`), `retry.rs`, `weather.rs`, `webhook.rs`
- **birdnet-migrate**: `provenance.rs`, `birdnet_pi/detector.rs`
- **birdnet-behavioral**: `gating.rs`, `phenology/types.rs`,
  `connection/{analytics,live,sync}.rs`
- **birdnet-scheduler**: `error.rs`

---

## Test Coverage

Counted as `#[test]` / `#[tokio::test]` attributes, which is also how the README's
test count is gated (`tests/documented_samples_match_the_build.rs`). It undercounts
anything a macro expands into many cases (`proptest!`, parameterised tests):

```bash
# one row per crate, then the binary and the integration tests
for d in crates/* src tests; do
  printf '%-28s %s\n' "$d" "$(grep -rhE --include='*.rs' '^\s*#\[(test|tokio::test)\]\s*$' "$d" | wc -l)"
done
# the total
grep -rhE --include='*.rs' '^\s*#\[(test|tokio::test)\]\s*$' src crates tests benches | wc -l
```

On 2026-10-20 the total was 5 030. The crates differ by an order of magnitude
(`birdnet-web` is the largest, `birdnet-scheduler` the smallest), so the loop is
the useful part; the single figure is only a reference point.

---

## Lines of Code

```bash
find src crates -name '*.rs' | xargs cat | wc -l
```

printed 276 926 on 2026-10-20 (comments and inline tests included; each crate's
`tests/` and `benches/` directories are inside `crates/`). Per crate, run the same
pipeline on `find crates/<name> -name '*.rs'`. Do not quote the figure without the
date.

---

## Key Dependencies

The version of each is in `Cargo.toml` under `[workspace.dependencies]` (the
requirement) and in `Cargo.lock` (what is built); this table says what each is
for and whether it is Rust all the way down.

| Purpose | Crate | Pure Rust |
|---------|-------|-----------|
| Web framework | `axum` | Yes |
| Async runtime | `tokio` | Yes |
| ONNX inference | `ort` (required as a release candidate, `^2.0.0-rc.N`) | No (C++ core, statically linked) |
| Audio decode | `symphonia` | Yes |
| Resampling | `rubato` | Yes |
| FFT | `realfft` | Yes |
| File watching | `notify` | Yes |
| Email (SMTP) | `lettre` | Yes (rustls TLS) |
| System monitoring | `sysinfo` | Yes |
| SSE streaming | `tokio-stream` | Yes |
| File streaming | `tokio-util` | Yes |
| SQLite | `rusqlite` | No (bundled C) |
| DuckDB | `duckdb` | No (bundled C++, optional) |
| CLI | `clap` | Yes |
| Serialization | `serde` + `serde_json` | Yes |
| Logging | `tracing` | Yes |

---

[← Risks](12-risks.md) | [Back to Index](../RUST_ARCHITECTURE_PLAN.md)
