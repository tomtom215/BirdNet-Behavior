# Implementation Status

> Current state of the Rust implementation as of March 2026.

## Summary

| Phase | Description | Status | Completion |
|-------|------------|--------|------------|
| 0 | Scaffolding | **Complete** | 100% |
| 1 | Data Layer | **Complete** | 100% |
| 2 | Audio Pipeline | **Partial** | 60% |
| 3 | ML Inference | Not started | 0% |
| 4 | Detection Daemon | Not started | 0% |
| 5 | Web Server | **Partial** | 70% |
| 6 | Integrations | **Partial** | 30% |
| 7 | Audio Capture | Not started | 0% |
| 8 | Assembly | **Partial** | 50% |

## Detailed Status by Crate

### birdnet-core

| Module | File | Status | Notes |
|--------|------|--------|-------|
| Config parser | `config.rs` | **Complete** | INI parsing, PHP quote stripping, full tests |
| Audio decode | `audio/decode.rs` | **Complete** | symphonia WAV/FLAC/MP3, mono downmix |
| Audio resample | `audio/resample.rs` | **Complete** | rubato polynomial resampler, chunked processing |
| Mel spectrogram | `audio/spectrogram.rs` | **Stubbed** | Critical for ML -- needs pure Rust implementation |
| Audio capture | `audio/capture.rs` | **Stubbed** | Subprocess management for arecord/ffmpeg |
| Detection types | `detection/types.rs` | **Complete** | Detection struct, RecordingFile parser, serde |
| Detection pipeline | `detection/pipeline.rs` | **Stubbed** | File watching → full inference chain |

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
| App state | `state.rs` | **Complete** | Arc<Mutex>, auto-migration on startup |
| Detection routes | `routes/detections.rs` | **Complete** | GET by date, recent, with limits |
| Species routes | `routes/species.rs` | **Complete** | Top species, hourly activity |
| System routes | `routes/system.rs` | **Complete** | Health, stats, API info |
| Analytics routes | `routes/analytics.rs` | **Placeholder** | Returns "planned" status |

### birdnet-integrations

| Module | File | Status | Notes |
|--------|------|--------|-------|
| BirdWeather | `birdweather.rs` | **Complete** | HTTP client, retry with exponential backoff |

### birdnet-behavioral

| Module | File | Status | Notes |
|--------|------|--------|-------|
| Result types | `types.rs` | **Complete** | All analytics result/param types defined |
| SQL builders | `queries.rs` | **Complete** | Sessionize, retention, funnel, patterns SQL |

### Binary

| Module | File | Status | Notes |
|--------|------|--------|-------|
| Entry point | `src/main.rs` | **Complete** | CLI, config, DB recovery, web server start |

## Next Priority Items

1. **Mel spectrogram** (Phase 2) -- Blocks everything downstream
2. **Detection pipeline** (Phase 4) -- Core functionality
3. **WebSocket endpoint** (Phase 5) -- Live detection streaming
4. **DuckDB integration** (Phase 1/8) -- Behavioral analytics

## Test Coverage

| Crate | Unit Tests | Status |
|-------|-----------|--------|
| birdnet-core | config, decode, resample, types | Passing |
| birdnet-db | sqlite, resilience, migration | Passing |
| birdnet-web | (integration via server) | Passing |
| birdnet-integrations | birdweather client | Passing |
| birdnet-behavioral | types, queries | Passing |

## Lines of Code (Rust, excluding tests)

| Crate | ~LOC | Notes |
|-------|------|-------|
| birdnet-core | ~500 | Excluding stubs |
| birdnet-db | ~600 | Full implementation |
| birdnet-web | ~400 | Excluding analytics |
| birdnet-integrations | ~100 | BirdWeather only |
| birdnet-behavioral | ~250 | Types + SQL builders |
| main.rs | ~150 | Entry point |
| **Total** | **~2,000** | Production Rust code |

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
| Core Pipeline | ~2,500 | 60% replaced |
| Web Server | ~7,000+ | 70% replaced |
| Shell Scripts | ~500 | Not started |
| Tests | ~19,000 | Rust tests written alongside implementation |
| **Total** | **~29,000** | **~2,000 LOC Rust replaces ~9,500 LOC Python** |

---

[← Risks](12-risks.md) | [Back to Index](../RUST_ARCHITECTURE_PLAN.md)
