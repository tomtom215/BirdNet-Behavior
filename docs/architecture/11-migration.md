# Migration Strategy

> Parallel running, backwards compatibility, and rollback plan.

## Parallel Running Period

During migration, Python and Rust components coexist safely because both use
SQLite WAL mode for concurrent read/write access.

```
Phase 1-2: Python analysis daemon + Rust DB layer
           (Rust writes to SQLite, Python reads — WAL makes this safe)

Phase 3-4: Rust analysis daemon + Rust DB layer
           Python web server still running

Phase 5:   Rust analysis daemon + Rust web server
           Python fully removed

Phase 6+:  Single binary, Python gone
```

## Backwards Compatibility

| Aspect | Compatibility |
|--------|--------------|
| SQLite database | Same schema, no migration needed |
| `birdnet.conf` | Same INI format, Rust parser handles PHP-style quotes |
| API endpoints | Same paths, drop-in replacement for FastAPI |
| systemd services | Same service names |
| Caddy config | Same reverse proxy config |
| BirdDB.txt CSV | Same format |
| Detection JSON | Same structure |

## Phase Execution Plan

### Phase 0: Scaffolding ✓ COMPLETE

- [x] Cargo workspace with 5 crates
- [x] CI with clippy + fmt + tests
- [x] `birdnet.conf` parser (INI-style with PHP quote stripping)
- [x] Cross-compilation toolchain verified

### Phase 1: Data Layer ✓ COMPLETE

- [x] `birdnet-db` with rusqlite (WAL, connection management)
- [x] Integrity checking, backup, corruption recovery
- [x] Schema migration framework (3 migrations)
- [x] Detection INSERT path
- [x] All read queries (by date, recent, top species, hourly activity)

### Phase 2: Audio Pipeline ⚠️ PARTIAL

- [x] WAV/FLAC/MP3 decoding via symphonia
- [x] Resampling via rubato (48kHz → model sample rate)
- [ ] **Mel spectrogram** -- critical missing piece
- [ ] Audio extraction (replaces sox trim)
- [ ] Spectrogram PNG generation

### Phase 3: ML Inference -- NOT STARTED

- [ ] Convert BirdNET TFLite model to ONNX
- [ ] Integrate `ort` crate (or `tract` for pure Rust)
- [ ] Validate predictions match Python output
- [ ] Benchmark on Pi 4/5
- [ ] Model hot-reload support

### Phase 4: Detection Daemon -- NOT STARTED

- [ ] File watcher via `notify` crate
- [ ] Full pipeline: watch → decode → spectrogram → infer → report
- [ ] Reporting: SQLite + CSV + JSON + web notification
- [ ] Graceful shutdown with in-flight completion
- [ ] Memory-bounded operation

### Phase 5: Web Server ⚠️ PARTIAL

- [x] axum server with CORS + tracing middleware
- [x] Graceful shutdown (SIGTERM/SIGINT)
- [x] REST API: detections, species, system endpoints
- [ ] WebSocket for live detections
- [ ] HTMX template rendering
- [ ] Static file embedding
- [ ] Authentication
- [ ] DuckDB analytics endpoints (actual queries)

### Phase 6: Integrations ⚠️ PARTIAL

- [x] BirdWeather client with retry logic
- [ ] Apprise notifications
- [ ] Flickr/Wikipedia image caching
- [ ] RTSP stream management
- [ ] Heartbeat monitoring

### Phase 7: Audio Capture -- NOT STARTED

- [ ] Subprocess management for arecord/ffmpeg
- [ ] Gap detection and alerting
- [ ] Automatic reconnection
- [ ] Disk space management

### Phase 8: Single Binary Assembly ⚠️ PARTIAL

- [x] Unified `main.rs` with CLI
- [x] Signal handling (SIGTERM, SIGINT)
- [x] Config validation on startup
- [x] Health check endpoint
- [ ] systemd service file
- [ ] Full subsystem integration

## Rollback Plan

At any phase, revert to Python:
1. Stop Rust binary: `systemctl stop birdnet-behavior`
2. Start Python services: `systemctl start birdnet_analysis birdnet_web`
3. Both use the same database and config files

No data migration needed in either direction.

---

[← Deployment](10-deployment.md) | [Back to Index](../RUST_ARCHITECTURE_PLAN.md) | [Next: Risks →](12-risks.md)
