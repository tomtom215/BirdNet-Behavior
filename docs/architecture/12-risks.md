# Risk Assessment

> Known risks, severity ratings, mitigation strategies, and resolved items.

## Table of Contents

- [Risk Matrix](#risk-matrix)
- [Critical Path Items](#critical-path-items)
- [Dependency Risks](#dependency-risks)
- [Operational Risks](#operational-risks)
- [Resolved Risks](#resolved-risks)
- [Outstanding Issues](#outstanding-issues)

---

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
| Community adoption barriers | **Low** | Medium | Ship as optional binary alongside existing Python install; migration UI simplifies onboarding |
| sysinfo API version churn | **Low** | Low | ✅ Resolved: pinned to 0.32 with correct API usage |

## Critical Path Items

These must succeed for the project to be viable:

1. **Mel spectrogram accuracy** — If Rust spectrograms don't match librosa,
   model predictions will be wrong. This is the highest-risk technical item.
   *Status: Not yet implemented; existing stub must be completed.*

2. **ONNX model conversion** — If tf2onnx produces models with different
   behavior, inference results won't match. Must validate all 4 model variants.
   *Status: Not yet started.*

3. **ARM64 inference performance** — If inference takes >2s per chunk on Pi 4,
   the system can't keep up with real-time audio.
   *Status: Not yet benchmarked.*

## Dependency Risks

| Dependency | Risk | Mitigation |
|-----------|------|------------|
| `ort` crate | ONNX Runtime version churn | Pin specific version; `tract` as fallback |
| `symphonia` | Pure Rust, well-maintained | Low risk |
| `rubato` | Pure Rust, specialized | Low risk |
| `rusqlite` | Wraps SQLite C; very stable | Low risk |
| `axum` | Tokio ecosystem, well-maintained | Low risk |
| `duckdb` crate | DuckDB version coupling | Pin version; DuckDB is optional for core function |
| `sysinfo` 0.32 | API changed between 0.31 and 0.32 | ✅ Pinned to 0.32; uses correct `RefreshKind::new()`, `component` feature |
| `lettre` | Pure Rust SMTP; well-maintained | Low risk |

## Operational Risks

| Risk | Mitigation |
|------|------------|
| Pi runs out of disk space | Disk management with automatic rotation (backup pruning implemented) |
| Database corruption from power loss | WAL mode + auto-recovery from backups (implemented) |
| Memory exhaustion on 1GB Pi 4 | Target <50MB RSS; bounded broadcast buffers (capacity=256); no accumulation |
| Network outage breaks BirdWeather | Retry queue with offline buffering (implemented) |
| Model file corruption | Checksum validation on load; refuse to start with bad model |
| Migration corrupts target database | All imports run in transactions; read-only access to source |
| Directory traversal in backup download | ✅ Resolved: canonical path check + filename allowlist |

## Resolved Risks

These risks were identified and fully mitigated:

### ✅ Raw string `"#` termination in SVG format strings

- **Problem**: `r#"..."#` is terminated by any `"#` sequence. CSS/SVG hex colors
  like `fill="#0f172a">` contain `"#`, silently truncating format strings.
- **Resolution**: All SVG/HTML format strings in the codebase upgraded to `r##"..."##`.
- **Prevention**: Documented in [Coding Standards](03-coding-standards.md).

### ✅ sysinfo 0.32 API incompatibilities

- **Problem**: `RefreshKind::nothing()` renamed to `RefreshKind::new()`;
  `Components` gated behind `component` feature; `refresh(true)` → `refresh()`.
- **Resolution**: Fixed in `system_info.rs`; workspace Cargo.toml updated with `features = ["system", "component"]`.

### ✅ Co-occurrence SQL double-counting

- **Problem**: Self-join for co-occurrence generates 2 rows per pair per date
  (A→B and B→A). `COUNT(*)` returned 2× the correct value.
- **Resolution**: Changed to `COUNT(DISTINCT a.Date)`. See [Database doc](07-database.md).

### ✅ `let_underscore_lock` lint

- **Problem**: `let _ = arc.db.lock()` holds the lock for the statement duration
  in older Rust; Clippy nursery correctly flags it.
- **Resolution**: `drop(arc.db.lock().ok())` for intentional lock-then-drop.

### ✅ Email settings `get_or` return type

- **Problem**: `get_or()` returns `Result<String, SettingsError>`, not `String`.
  14 type errors when building `EmailConfig` from settings.
- **Resolution**: Inner helper `fn s(r: Result<String, SettingsError>, default: &str) -> String`
  with fallback-on-error semantics.

### ✅ Migration 3-tuple destructure

- **Problem**: `validate_source` returns 3-tuple `(SchemaInfo, SourceReport, MigrationReport)`;
  tests expected 2-tuple.
- **Resolution**: Tests updated to `let (schema, report, _migration_report) = ...`.

## Outstanding Issues

1. **FP16 model handling** — BirdNET V2.4 uses FP16 weights. Need to verify
   that ONNX Runtime (and tract) handle FP16→FP32 conversion correctly on ARM.

2. **Timezone handling** — Detection timestamps must match the station's local
   timezone. Python uses `tzlocal`; Rust needs equivalent handling. Currently
   timestamps are stored as-is from the detection pipeline.

3. **Large dataset performance** — Stations running for years may accumulate
   millions of detections. SQLite query performance and DuckDB sync performance
   at scale need validation.

4. **Concurrent database writes under load** — WAL mode handles concurrent
   readers + one writer, but high-frequency detection events plus web requests
   need load testing to verify no lock contention.

5. **RTSP stream reconnection** — The RTSP stub needs full subprocess management
   with gap detection, reconnection backoff, and disk space monitoring before
   it can replace the Python `birdnet_recording.sh` script.

6. **Model selection UI** — Multiple BirdNET model files should be selectable
   from the admin settings page. Currently the model path is a config value only.

7. **duckdb-behavioral extension loading** — The behavioral analytics functions
   (sessionization, retention, funnel) require the duckdb-behavioral extension
   to be loaded. The loading mechanism is not yet wired up.

---

*Last updated: 2026-03-14*

[← Migration](11-migration.md) | [Back to Index](../RUST_ARCHITECTURE_PLAN.md) | [Next: Implementation Status →](13-implementation-status.md)
