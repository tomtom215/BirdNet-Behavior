# Risk Assessment

> Known risks, severity ratings, and mitigation strategies.

## Table of Contents

- [Risk Matrix](#risk-matrix)
- [Dependency Risks](#dependency-risks)
- [Operational Risks](#operational-risks)
- [Outstanding Issues](#outstanding-issues)

---

## Risk Matrix

| Risk | Severity | Likelihood | Mitigation |
|------|----------|-----------|------------|
| Mel spectrogram divergence from librosa | Medium | Low | Pure-Rust implementation validated against librosa reference spectrograms within 1e-4 tolerance; covered by unit tests |
| ONNX model accuracy loss | Medium | Low | End-to-end tests run a real WAV fixture through the full pipeline and assert expected species at expected confidence |
| DuckDB ARM64 performance under load | Medium | Medium | Analytics feature is optional; operational queries stay on SQLite; DuckDB queries are synced from SQLite and tuned for columnar access |
| Cross-compilation drift between release and Docker images | Low | Low | Release binaries use `cargo-zigbuild`; Docker images build natively per architecture on matching runners |
| RTSP / audio capture subprocess instability | Low | Low | Supervised `ffmpeg` / `arecord` subprocesses with restart logic, gap detection, and disk monitoring |

## Dependency Risks

| Dependency | Risk | Mitigation |
|-----------|------|------------|
| `ort` | ONNX Runtime version churn and large native dependency | Pinned version; release images statically link ONNX Runtime so deployed binaries have no runtime ONNX Runtime dependency |
| `symphonia`, `rubato`, `realfft` | Pure Rust, actively maintained | Low risk |
| `rusqlite` (bundled) | Wraps SQLite C; very stable | Low risk |
| `duckdb` | C++ build cost and version coupling | Optional `analytics` feature; native per-architecture Docker builds avoid emulation; `Cargo.lock` pinned |
| `axum` / `tokio` | Active, well-maintained ecosystem | Low risk |
| `lettre` | Pure Rust SMTP with rustls | Low risk |
| `sysinfo` | API churn between minor versions | Pinned to 0.32; workspace manifest enables the `system` and `component` features explicitly |

## Operational Risks

| Risk | Mitigation |
|------|------------|
| Disk exhaustion on long-running stations | Disk manager with automatic rotation, per-species file caps, and disk-usage thresholds |
| Database corruption from power loss | WAL mode, integrity checks at startup, auto-recovery from the most recent backup |
| Memory exhaustion on 1 GB Pi 4 | Target <50 MB RSS; bounded broadcast channels (`capacity = 256`); no unbounded accumulation |
| Network outage blocking BirdWeather uploads | Exponential backoff with offline buffering |
| Model file corruption or missing weights | Loader refuses to start with an invalid model; the Docker entrypoint re-downloads the model on first run |
| Migration corrupts target database | Imports run inside a single transaction; source is opened read-only |
| Directory traversal on backup download | Canonical path check + filename allow-list |
| Unauthorised web access | Optional HTTP Basic Auth with constant-time comparison; per-IP token-bucket rate limiter on API and admin routes |

## Outstanding Issues

These are known open items tracked for future releases:

1. **FP16 model handling on ARM** — BirdNET V2.4 ships FP16 weights. ONNX
   Runtime's ARM backend converts to FP32 at load, but end-to-end
   accuracy on FP16 models should be spot-checked on a real Pi.
2. **Large dataset performance** — Stations running for years may
   accumulate millions of detections. Operational queries are bounded
   (paginated, time-windowed); DuckDB sync throughput at scale still
   warrants a dedicated benchmark suite.
3. **Concurrent database contention under load** — WAL mode allows
   concurrent readers plus one writer. High-frequency detection events
   combined with web requests should be load-tested to confirm no lock
   contention emerges at the upper end of realistic station throughput.

---

[← Migration](11-migration.md) | [Back to Index](../RUST_ARCHITECTURE_PLAN.md) | [Next: Implementation Status →](13-implementation-status.md)
