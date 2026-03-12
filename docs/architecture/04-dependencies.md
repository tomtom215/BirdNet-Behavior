# Dependencies

> Minimal dependency philosophy: pure Rust where possible, C bindings only when necessary.

## Guiding Principles

1. **Pure Rust first** -- avoid C/C++ dependencies that complicate cross-compilation
2. **Minimal surface area** -- every dependency is an attack surface and maintenance burden
3. **Hand-roll simple things** -- don't add a crate for <100 lines of code
4. **Pin for reproducibility** -- exact versions for system-critical deps
5. **Audit everything** -- `cargo-deny` for licenses, advisories, sources

## Core Dependencies

### Pure Rust (Zero C Dependencies)

| Purpose | Crate | Notes |
|---------|-------|-------|
| Audio decode | `symphonia` | WAV/FLAC/MP3, pure Rust, royalty-free codecs |
| Resampling | `rubato` | High-quality async polynomial resampling |
| File watching | `notify` | Cross-platform inotify wrapper |
| Serialization | `serde` + `serde_json` | Derive-based, zero-overhead |
| Logging | `tracing` | Structured, async-aware |
| CLI | `clap` | Argument parsing with derive |
| Config parsing | `configparser` | INI-style parser |
| HTTP client | `reqwest` | BirdWeather API calls |
| Web framework | `axum` | Tower-based, minimal |
| Async runtime | `tokio` | Only in web/app crates |
| Middleware | `tower`, `tower-http` | CORS, tracing, static files |

### C-Binding Dependencies (Necessary)

| Purpose | Crate | Why C binding is needed |
|---------|-------|----------------------|
| SQLite | `rusqlite` (bundled) | Bundles SQLite C source; no system dependency required |
| ML inference | `ort` | ONNX Runtime for BirdNET model inference |

### ML Inference Strategy

| Option | Crate | Cross-compile | Pure Rust | Recommendation |
|--------|-------|---------------|-----------|---------------|
| ONNX Runtime | `ort` v2 | Medium (pre-built aarch64) | No (C++ core) | **Primary** -- production-proven, ARM NEON acceleration |
| Tract | `tract` | Trivial | **Yes** | **Long-term goal** -- pure Rust, evaluate TFLite/ONNX support |
| TFLite FFI | `tflite` | Hard (Bazel + TF) | No | **Avoid** -- cross-compile nightmare |
| TFLite C | `tflitec` | Medium | No | **Avoid** -- pinned to outdated TF 2.9.1 |

**Primary choice: `ort`** (ONNX Runtime). Used by SurrealDB, Google Magika, Bloop.
Pre-built aarch64 binaries from Microsoft are auto-downloaded.

**Long-term aspiration: `tract`** (pure Rust). If `tract` can run the BirdNET ONNX
model with equivalent accuracy, it would eliminate the last C++ dependency from
the inference pipeline and make cross-compilation trivial.

### Model Conversion

Convert BirdNET TFLite models to ONNX:
```bash
pip install tf2onnx onnxruntime
python -m tf2onnx.convert --tflite BirdNET_model.tflite --output BirdNET_model.onnx --opset 18
```

### Dependencies NOT Used (And Why)

| Crate | Reason for exclusion |
|-------|---------------------|
| `anyhow` | Banned from library crates; hand-rolled errors are more precise |
| `thiserror` | Derive macro adds compile-time cost; hand-rolling is simple enough |
| `r2d2` / `deadpool` | Connection pooling not needed; single connection with `Arc<Mutex>` suffices for embedded use |
| `askama` / `minijinja` | Template engine deferred; HTMX can work with simple string formatting initially |
| `image` | Spectrogram PNG generation deferred; not needed for core detection pipeline |
| `cpal` | Direct audio capture deferred; subprocess `arecord`/`ffmpeg` is simpler and proven |

## Dependency Count by Crate

Target direct dependency counts (excluding `serde` and `tracing` which are universal):

| Crate | Direct deps | Notes |
|-------|------------|-------|
| `birdnet-core` | 4 | symphonia, rubato, notify, configparser |
| `birdnet-db` | 1 | rusqlite |
| `birdnet-web` | 4 | axum, tower, tower-http, tokio |
| `birdnet-integrations` | 2 | reqwest, tokio |
| `birdnet-behavioral` | 0 | Types and SQL builders only |

## Supply Chain Security

All dependencies audited via `cargo-deny`:
- **Licenses**: Only permissive licenses (MIT, Apache-2.0, BSD)
- **Advisories**: RustSec advisory database checked in CI
- **Sources**: Only crates.io (no git dependencies in production)
- **Bans**: No duplicate versions of critical crates

---

[← Coding Standards](03-coding-standards.md) | [Back to Index](../RUST_ARCHITECTURE_PLAN.md) | [Next: Audio Pipeline →](05-audio-pipeline.md)
