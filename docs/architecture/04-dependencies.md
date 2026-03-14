# Dependencies

> Minimal dependency philosophy: pure Rust where possible, C bindings only when necessary.

## Table of Contents

- [Guiding Principles](#guiding-principles)
- [Core Dependencies](#core-dependencies)
- [ML Inference Strategy](#ml-inference-strategy)
- [Dependencies NOT Used](#dependencies-not-used-and-why)
- [Actual Dependency Count by Crate](#actual-dependency-count-by-crate)
- [Supply Chain Security](#supply-chain-security)

---

## Guiding Principles

1. **Pure Rust first** — avoid C/C++ dependencies that complicate cross-compilation
2. **Minimal surface area** — every dependency is an attack surface and maintenance burden
3. **Hand-roll simple things** — don't add a crate for <100 lines of code
4. **Pin for reproducibility** — exact versions for system-critical deps
5. **Audit everything** — `cargo-deny` for licenses, advisories, sources

## Core Dependencies

### Pure Rust (Zero C Dependencies)

| Purpose | Crate | Notes |
|---------|-------|-------|
| Audio decode | `symphonia` | WAV/FLAC/MP3, pure Rust, royalty-free codecs |
| Resampling | `rubato` | High-quality async polynomial resampling |
| File watching | `notify` | Cross-platform inotify wrapper |
| Serialization | `serde` + `serde_json` | Derive-based, zero-overhead |
| Logging | `tracing` + `tracing-subscriber` | Structured, async-aware |
| CLI | `clap` | Argument parsing with derive |
| Config parsing | `configparser` | INI-style parser for birdnet.conf |
| HTTP client | `reqwest` (rustls) | BirdWeather API calls; no OpenSSL |
| Web framework | `axum` 0.8 | Tower-based, minimal |
| Async runtime | `tokio` | Only in web/app crates |
| Middleware | `tower`, `tower-http` | CORS, tracing, static files |
| Async streaming | `tokio-util` | `ReaderStream` for file downloads, SSE |
| Email (SMTP) | `lettre` (rustls) | Pure Rust SMTP client; no OpenSSL |
| System info | `sysinfo` 0.32 | CPU/memory/disk metrics for admin panel |

### C-Binding Dependencies (Necessary)

| Purpose | Crate | Why C binding is needed |
|---------|-------|----------------------|
| SQLite | `rusqlite` (bundled) | Bundles SQLite C source; no system dependency required |
| ML inference | `ort` | ONNX Runtime for BirdNET model inference |

### Key Version Notes

- **sysinfo 0.32**: `RefreshKind::new()` (not `nothing()`); `Components` gated behind `component` feature;
  `components.refresh()` takes 0 arguments (not `refresh(true)`)
- **axum 0.8**: Uses `IntoResponse`, `Router::merge`, `extract::Path`, `extract::State`
- **lettre**: Configured with `SmtpsTransport` or `StarttlsRelay`; no system OpenSSL needed

## ML Inference Strategy

| Option | Crate | Cross-compile | Pure Rust | Recommendation |
|--------|-------|---------------|-----------|---------------|
| ONNX Runtime | `ort` v2.0.0-rc | Medium (pre-built aarch64) | No (C++ core) | **Primary** — production-proven, ARM NEON |
| Tract (pure Rust) | `tract-onnx` 0.22 | Trivial | **Yes** | **Preferred** if accuracy validates |
| ort-tract bridge | `ort-tract` | Trivial | **Yes** | Use ort API with tract backend |
| TFLite FFI | `tflite` | Hard (Bazel + TF) | No | **Avoid** — cross-compile nightmare |
| TFLite C | `tflitec` | Medium | No | **Avoid** — pinned to outdated TF 2.9.1 |

**Preferred approach: `tract-onnx`** (pure Rust, Sonos). `tract` passes ~85% of
ONNX backend tests but handles common operators well. For BirdNET's relatively
simple model (conv + dense layers), this should be sufficient. Pure Rust means
zero cross-compilation issues and smaller binaries.

**Bridge option: `ort-tract`** provides the `ort` API surface with `tract` as
the backend. This allows starting with the ort API and swapping backends later.

**Fallback: `ort`** v2.0.0-rc (ONNX Runtime). Used by SurrealDB, Google Magika.
For aarch64, you must supply ONNX Runtime binaries manually via `ORT_LIB_PATH`
(Microsoft provides official aarch64 Linux builds, requires glibc >= 2.35).

**Validation required:** Run the converted BirdNET ONNX model through both
`tract` and `ort`, compare outputs against Python TFLite on identical inputs.
If tract matches within 1e-4, prefer it for the pure Rust deployment.

### Model Conversion

Convert BirdNET TFLite models to ONNX:
```bash
pip install tf2onnx onnxruntime
python -m tf2onnx.convert --tflite BirdNET_model.tflite --output BirdNET_model.onnx --opset 18
```

## Dependencies NOT Used (And Why)

| Crate | Reason for exclusion |
|-------|---------------------|
| `anyhow` | Banned from library crates; hand-rolled errors are more precise |
| `thiserror` | Derive macro adds compile-time cost; hand-rolling is simple enough |
| `r2d2` / `deadpool` | Connection pooling not needed; single connection with `Arc<Mutex>` suffices for embedded use |
| `askama` / `minijinja` | Template engine avoided; HTMX works with format strings; keeps binary smaller |
| `image` | Spectrogram PNG generation deferred; not needed for core detection pipeline |
| `cpal` | Direct audio capture avoided; subprocess `arecord`/`ffmpeg` is simpler and proven |
| `chrono` | Pure-Rust timestamp formatting hand-rolled for Unix → date conversion; avoids chrono's known time zone complexity |
| `openssl` | All TLS done via `rustls` (in reqwest and lettre); no system OpenSSL dependency |

## Actual Dependency Count by Crate

Direct dependencies (excluding universal `serde` and `tracing`):

| Crate | Direct deps | Key deps |
|-------|------------|---------|
| `birdnet-core` | 4 | symphonia, rubato, notify, configparser |
| `birdnet-db` | 2 | rusqlite, duckdb |
| `birdnet-web` | 7 | axum, tower, tower-http, tokio, tokio-util, sysinfo, serde_json |
| `birdnet-integrations` | 3 | reqwest, tokio, lettre |
| `birdnet-behavioral` | 0 | Types and SQL builders only |
| `birdnet-timeseries` | 0 | Pure math, no external deps |
| `birdnet-migrate` | 2 | rusqlite, birdnet-db |

## Supply Chain Security

All dependencies audited via `cargo-deny`:
- **Licenses**: Only permissive licenses (MIT, Apache-2.0, BSD)
- **Advisories**: RustSec advisory database checked in CI
- **Sources**: Only crates.io (no git dependencies in production)
- **Bans**: No duplicate versions of critical crates

---

*Last updated: 2026-03-14*

[← Coding Standards](03-coding-standards.md) | [Back to Index](../RUST_ARCHITECTURE_PLAN.md) | [Next: Audio Pipeline →](05-audio-pipeline.md)
