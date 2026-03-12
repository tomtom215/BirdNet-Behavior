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
| 5 | Web Server | **Complete** | 90% |
| 6 | Integrations | **Partial** | 30% |
| 7 | Audio Capture | **Complete** | 100% |
| 8 | Assembly | **Complete** | 90% |

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
| App state | `state.rs` | **Complete** | Arc<Mutex>, auto-migration, detection broadcast |
| Detection routes | `routes/detections.rs` | **Complete** | GET by date, recent, with limits |
| Species routes | `routes/species.rs` | **Complete** | Top species, hourly activity |
| System routes | `routes/system.rs` | **Complete** | Health, stats, API info |
| Analytics routes | `routes/analytics.rs` | **Placeholder** | Returns "planned" status (awaits DuckDB) |
| WebSocket | `routes/websocket.rs` | **Complete** | Live detection streaming, broadcast, ping/pong |

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
| Entry point | `src/main.rs` | **Complete** | CLI, config, DB recovery, detection daemon, WebSocket bridge |

## Recent Changes (March 12, 2026)

### ML Inference Module (Phase 3 — Complete)

- Pure Rust ONNX inference via `tract-onnx` v0.22.1 (zero C dependencies)
- Supports BirdNET V2.4 FP16, V1, and Perch V2 model input shapes
- `sigmoid()` and `softmax()` activation functions
- Configurable sensitivity, confidence threshold, top-N results
- Species label parser for BirdNET `"Scientific_Common"` format
- 14 unit tests for model loading, prediction, config, edge cases

### Detection Daemon (Phase 4 — Complete)

- File watcher daemon using `notify` crate on background thread
- Processes new WAV/FLAC/MP3 files through full pipeline → inference
- Event broadcasting via `std::mpsc` channel to main binary
- `DaemonHandle` for graceful shutdown
- Optional processing of pre-existing files on startup
- CLI flags: `--model`, `--labels`, `--watch-dir`, `--process-existing`

### WebSocket Live Streaming (Phase 5 — Complete)

- `DetectionBroadcast` wrapping `tokio::broadcast::Sender<Arc<String>>`
- WebSocket endpoint at `/api/v2/ws` with welcome message
- Automatic ping/pong keepalive (30s interval)
- Lag detection — drops behind subscribers gracefully
- Event processor bridges daemon → DB insert → WebSocket broadcast
- 4 unit tests for broadcast behavior

### Integration Tests

- 19 audio pipeline integration tests (15 synthetic + 4 real Pica pica)
- 8 web API integration tests (root, health, stats, detections, species, activity, analytics)
- Real audio test uses Pica pica (Eurasian Magpie) 30s WAV from BirdNET-Pi test suite
- Full pipeline benchmark: ~4s for 30s audio, 10 chunks at 128 mel bands

## Next Priority Items

1. **DuckDB integration** — Connect behavioral analytics queries to real database
2. **HTMX web UI** — Server-rendered bird detection dashboard
3. **Apprise notifications** — Push alerts for rare species
4. **Flickr/Wikipedia** — Species image caching

## Test Coverage

| Crate | Unit Tests | Integration Tests | Status |
|-------|-----------|------------------|--------|
| birdnet-core | 54 (config, decode, resample, spectrogram, labels, model, pipeline, daemon) | 19 (audio pipeline + real Pica pica) | All passing |
| birdnet-db | 19 (sqlite, resilience, migration) | — | All passing |
| birdnet-web | 4 (websocket broadcast) | 8 (HTTP API endpoints) | All passing |
| birdnet-integrations | 2 (birdweather client) | — | All passing |
| birdnet-behavioral | 10 (types, queries) | — | All passing |
| **Total** | **89** | **27** | **116 tests passing** |

## Lines of Code (Rust, excluding tests)

| Crate | ~LOC | Notes |
|-------|------|-------|
| birdnet-core | ~1,200 | Audio pipeline + inference + daemon |
| birdnet-db | ~600 | Full implementation |
| birdnet-web | ~500 | REST API + WebSocket |
| birdnet-integrations | ~150 | BirdWeather only |
| birdnet-behavioral | ~250 | Types + SQL builders |
| main.rs | ~370 | Entry point + daemon bridge |
| Integration tests | ~620 | audio_pipeline.rs + web_api.rs |
| **Total** | **~3,700** | Production Rust code |

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
| Web Server | ~7,000+ | 80% replaced |
| Shell Scripts | ~500 | Not started |
| Tests | ~19,000 | Rust tests written alongside implementation |
| **Total** | **~29,000** | **~3,700 LOC Rust replaces ~9,500 LOC Python** |

---

[← Risks](12-risks.md) | [Back to Index](../RUST_ARCHITECTURE_PLAN.md)
