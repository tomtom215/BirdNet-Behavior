# Dependencies

> Minimal dependency philosophy: pure Rust where possible, C bindings only when necessary.

## Table of Contents

- [Guiding Principles](#guiding-principles)
- [Core Dependencies](#core-dependencies)
- [ML Inference Runtime](#ml-inference-runtime)
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
| DuckDB | `duckdb` (bundled, optional) | Bundles DuckDB C++ source behind the `analytics` feature; roughly seven minutes of C++ compilation on first build |
| ML inference | `ort` (rustls) | ONNX Runtime for BirdNET model inference; uses rustls for the binary download step |

### Notes on specific versions

- **sysinfo 0.32** — `Components` is gated behind the `component` feature;
  the workspace manifest enables `features = ["system", "component"]`.
- **axum 0.8** — the routing API uses `IntoResponse`, `Router::merge`,
  `extract::Path`, and `extract::State`.
- **lettre** — configured with `SmtpsTransport` or `StarttlsRelay` using
  the `tokio1-rustls-tls` feature; no system OpenSSL is needed.

## ML Inference Runtime

Inference is performed by the [`ort`](https://crates.io/crates/ort) crate —
a Rust binding for Microsoft's ONNX Runtime — configured with the
`download-binaries`, `tls-rustls`, and `copy-dylibs` features. This keeps
the runtime self-contained: there is no system `libonnxruntime` dependency
and no OpenSSL.

| Crate | Version | Notes |
|-------|---------|-------|
| `ort`      | `2.0.0-rc` | ONNX Runtime wrapper; handles session management, optimization levels, threading |
| `ndarray`  | `0.16` | Tensor inputs and outputs for the session |

**Cross-compilation.** `ort` fetches pre-built ONNX Runtime binaries for
`aarch64-unknown-linux-gnu` and `x86_64-unknown-linux-gnu` automatically,
which makes `cargo build --target` work out of the box on GitHub Actions
runners. Release images produced by `.github/workflows/docker.yml` build
natively on `ubuntu-24.04` and `ubuntu-24.04-arm` runners to avoid QEMU
emulation, and statically link ONNX Runtime into the final binary.

**Model format.** BirdNET models are distributed as ONNX (converted from
TFLite upstream via `tf2onnx`). The model file path is passed via
`BIRDNET_MODEL` / `--model` at startup. No runtime conversion is needed.

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
| `openssl` | All TLS done via `rustls` (in reqwest, lettre, and ort); no system OpenSSL dependency |

## Actual Dependency Count by Crate

Direct dependencies (excluding universal `serde` and `tracing`):

| Crate | Key direct dependencies |
|-------|-------------------------|
| `birdnet-core` | symphonia, rubato, realfft, notify, configparser, ort, ndarray, hound |
| `birdnet-db` | rusqlite |
| `birdnet-web` | axum, tower, tower-http, tokio, tokio-util, tokio-stream, sysinfo, reqwest |
| `birdnet-integrations` | reqwest, tokio, lettre |
| `birdnet-behavioral` | duckdb + rusqlite (optional, `analytics` feature) |
| `birdnet-timeseries` | duckdb (optional, `analytics` feature) |
| `birdnet-migrate` | rusqlite, birdnet-db |
| `birdnet-scheduler` | (serde + tracing only) |

## Supply Chain Security

- **Licenses** — workspace standardises on permissive dependencies (MIT, Apache-2.0, BSD, MPL)
- **TLS** — every HTTPS client (`reqwest`, `lettre`, `ort`) is configured to use `rustls`; there is no system OpenSSL dependency
- **Sources** — all dependencies come from crates.io; no git dependencies in production
- **`Cargo.lock`** — committed to the repository for reproducible builds across CI and release

---

[← Coding Standards](03-coding-standards.md) | [Back to Index](../RUST_ARCHITECTURE_PLAN.md) | [Next: Audio Pipeline →](05-audio-pipeline.md)
