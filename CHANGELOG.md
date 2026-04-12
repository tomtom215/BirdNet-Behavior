# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-04-12

First public release. BirdNet-Behavior is a ground-up Rust rewrite of
BirdNET-Pi that ships as a single static binary for Raspberry Pi and
x86_64 Linux.

### Added

#### Core detection pipeline

- Pure-Rust audio pipeline with `symphonia` (decode), `rubato` (resampling),
  and `realfft` (mel spectrogram) — zero C dependencies in the audio path.
- ONNX Runtime inference through the `ort` crate, statically linked into
  release binaries. BirdNET+ V3.0 is the default model; BirdNET V2.4 FP16
  and V1 remain compatible.
- File-watcher detection daemon with configurable chunking, overlap,
  sensitivity, per-species confidence thresholds, and privacy filtering.
- Audio quality pre-filtering: SNR estimation, spectral flatness,
  adaptive noise-floor tracking, and rain / wind detection.
- Species occurrence frequency filter driven by the BirdNET metadata
  model, with whitelist, include, and exclude lists.
- Rare-bird quarantine workflow: detections that fall below per-species
  thresholds are quarantined for manual review rather than dropped.

#### Audio capture

- ALSA, PulseAudio, PipeWire, and RTSP capture sources, each supervised
  as a restart-aware subprocess with gap detection and disk monitoring.
- Multiple simultaneous RTSP streams via `--rtsp-urls`.
- Solar-aware recording scheduler with sunrise / sunset computation,
  twilight offsets, fixed-window schedules, and a night-inhibit mode.
- tmpfs support for transient audio storage to reduce SD card wear on
  Raspberry Pi deployments.
- Automatic disk management: per-species retention caps, auto-purge, and
  configurable disk-usage thresholds.

#### Storage and resilience

- SQLite operational database with WAL mode, ten idempotent schema
  migrations, integrity checks, hot backup, restore, and auto-recovery.
- Per-IP rate limiter on API and admin routes (token-bucket with
  `Retry-After` header).
- HTTP Basic Auth with constant-time comparison.

#### Web server and dashboard

- `axum` HTTP server with REST API, WebSocket, Server-Sent Events, and
  server-rendered HTMX pages. No client-side JavaScript framework.
- HTMX pages: dashboard, today, history, species list, species detail,
  species gallery, life list, activity heatmap, correlation, charts,
  weekly report, recordings browser, audio player, livestream, kiosk,
  notification center, quarantine, system health, and weekly report.
- Admin panel: settings editor, species thresholds, species filter
  tester, BirdNET-Pi migration wizard, system info, backup management,
  live log viewer (SSE), notification history, alert rules, data
  quality dashboard, and binary update check.
- Full dark / light theme support with OS preference detection.

#### Analytics (optional `analytics` feature)

- DuckDB behavioral analytics: sessionize, retention, funnel, sequence,
  and next-species prediction, implemented via the duckdb-behavioral
  extension.
- Phenology analytics: migration timing percentiles, weekly abundance
  index, peak weeks, monthly totals, species richness, and
  effort-corrected abundance.
- Time-series analytics: activity, diversity (Shannon), trend, peak,
  gap, and session windows (tumbling, sliding, hopping, session).

#### Integrations

- BirdWeather detection and soundscape uploads with retry and backoff.
- Apprise notifications across 80+ channels with per-species cooldown,
  watchlist, and template rendering.
- SMTP email alerts via `lettre` with rustls TLS (no OpenSSL).
- Wikipedia species image cache with on-disk and in-memory indexing.
- Pure-Rust MQTT 3.1.1 publisher (no external broker library) with
  Home Assistant auto-discovery.
- GitHub Releases auto-update with atomic binary replacement.
- Heartbeat URL pinging for uptime monitors.

#### Migration

- Non-destructive BirdNET-Pi import wizard. Source database is opened
  read-only. Transactional, idempotent, with pre- and post-migration
  species reports and a data quality report.
- Supports both BirdNET-Pi SQLite databases and `BirdDB.txt` CSV flat
  files.

#### Observability and deployment

- Prometheus metrics endpoint (`/api/v2/metrics`).
- `tracing`-based structured logging with SSE log streaming.
- Multi-architecture Docker images published to GHCR (`linux/amd64`,
  `linux/arm64`), with and without the `analytics` feature.
- Cross-compiled release binaries for `aarch64-unknown-linux-gnu` and
  `x86_64-unknown-linux-gnu`.  The `ort` crate does not ship prebuilt
  ONNX Runtime binaries for `armv7-unknown-linux-gnueabihf`, so 32-bit
  ARM is not supported — Pi 3 / Pi Zero 2W users should install the
  64-bit Raspberry Pi OS, or build from source.
- Release binaries are built on Ubuntu 24.04 (GCC 13, glibc 2.39) to
  match the libstdc++ and glibc baselines that pyke's prebuilt ONNX
  Runtime archives require.  **Runtime requirement: glibc >= 2.39**
  (Raspberry Pi OS Trixie, Debian 13, Ubuntu 24.04, or newer).
- systemd installer script with ALSA microphone auto-detection and
  automatic BirdNET+ model download from Zenodo.

[Unreleased]: https://github.com/tomtom215/BirdNet-Behavior/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/tomtom215/BirdNet-Behavior/releases/tag/v0.1.0
