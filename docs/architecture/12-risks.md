# Risk Assessment

> Known risks, severity ratings, and mitigation strategies.

## Risk Matrix

| Risk | Severity | Likelihood | Mitigation |
|------|----------|-----------|------------|
| ONNX model conversion loses accuracy | **High** | Medium | Validate predictions match Python ±0.01 confidence; test all 4 model variants |
| Mel spectrogram differences affect inference | **High** | Medium | Bit-accurate comparison tests with librosa; 1e-4 tolerance validation |
| Pure Rust mel spectrogram doesn't match librosa | **High** | Low | Use `realfft` for FFT (same algorithm as numpy); validate HTK mel scale matches exactly |
| DuckDB ARM64 performance untested | **Medium** | Medium | Benchmark early; fallback to SQLite-only analytics if needed |
| Cross-compilation with ONNX Runtime | **Medium** | Medium | Pre-built aarch64 binaries available; `tract` as pure Rust fallback |
| DuckDB C++ cross-compilation complexity | **Medium** | High | Custom `cross` Docker image; or native build on Pi 5 |
| RTSP/audio capture subprocess management | **Low** | Low | Keep ffmpeg subprocess; well-understood pattern from Python codebase |
| Web template migration effort | **Low** | Low | Start with JSON API; HTMX templates are mechanical translation from Jinja2 |
| Community adoption barriers | **Low** | Medium | Ship as optional binary alongside existing Python install |

## Critical Path Items

These must succeed for the project to be viable:

1. **Mel spectrogram accuracy** -- If Rust spectrograms don't match librosa,
   model predictions will be wrong. This is the highest-risk technical item.

2. **ONNX model conversion** -- If tf2onnx produces models with different
   behavior, inference results won't match. Must validate all 4 model variants.

3. **ARM64 inference performance** -- If inference takes >2s per chunk on Pi 4,
   the system can't keep up with real-time audio.

## Dependency Risks

| Dependency | Risk | Mitigation |
|-----------|------|------------|
| `ort` crate | ONNX Runtime version churn | Pin specific version; `tract` as fallback |
| `symphonia` | Pure Rust, well-maintained | Low risk |
| `rubato` | Pure Rust, specialized | Low risk |
| `rusqlite` | Wraps SQLite C; very stable | Low risk |
| `axum` | Tokio ecosystem, well-maintained | Low risk |
| `duckdb` crate | DuckDB version coupling | Pin version; DuckDB is optional for core function |

## Operational Risks

| Risk | Mitigation |
|------|------------|
| Pi runs out of disk space | Disk management with automatic rotation (from Python codebase) |
| Database corruption from power loss | WAL mode + auto-recovery from backups |
| Memory exhaustion on 1GB Pi 4 | Target <50MB RSS; bounded buffers; no accumulation |
| Network outage breaks BirdWeather | Retry queue with offline buffering (implemented) |
| Model file corruption | Checksum validation on load; refuse to start with bad model |

## What Could Go Wrong (and Hasn't Been Addressed Yet)

1. **FP16 model handling** -- BirdNET V2.4 uses FP16 weights. Need to verify
   that ONNX Runtime (and tract) handle FP16→FP32 conversion correctly on ARM.

2. **Timezone handling** -- Detection timestamps must match the station's local
   timezone. Python uses `tzlocal`; Rust needs equivalent handling.

3. **Concurrent database writes** -- If analysis daemon and web server both
   write to SQLite simultaneously, WAL mode handles it, but need to verify
   no lock contention under load.

4. **Large dataset performance** -- Stations running for years may accumulate
   millions of detections. SQLite query performance and DuckDB sync performance
   at scale need validation.

---

[← Migration](11-migration.md) | [Back to Index](../RUST_ARCHITECTURE_PLAN.md) | [Next: Implementation Status →](13-implementation-status.md)
