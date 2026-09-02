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
| Config parsing | *(none — hand-rolled)* | `birdnet.conf` is `KEY=VALUE` with `#` comments and no sections; `birdnet-core::config::Config::load_from` parses it in about thirty lines rather than taking a crate for it |
| HTTP client | `reqwest` (rustls) | BirdWeather API calls; no OpenSSL |
| Web framework | `axum` 0.8 | Tower-based, minimal |
| Async runtime | `tokio` | Only in web/app crates |
| Middleware | `tower`, `tower-http` | CORS, tracing, static files |
| Async streaming | `tokio-util` | `ReaderStream` for file downloads, SSE |
| Email (SMTP) | `lettre` (rustls) | Pure Rust SMTP client; no OpenSSL |
| Dashboard HTTPS | `tokio-rustls`, `hyper`, `hyper-util`, `rustls-pki-types` | `--tls-mode` serves the dashboard directly. `axum::serve` cannot wrap a TLS acceptor, so the accept loop runs over hyper-util's h1+h2 auto builder. PEM parsing is `rustls_pki_types::pem` — `rustls-pemfile` was archived in August 2025 (RUSTSEC-2025-0134) and wrapped the same parser |
| Self-signed certificates | `rcgen` (ring) | Generates the local CA and the leaf it signs for `--tls-mode self-signed` |
| Backup encryption | `ring` (`aead`, `rand`) | ChaCha20-Poly1305 for the offsite backup envelope. Already compiled as rustls's provider, so a direct edge rather than a new crate |
| Backup key derivation | `argon2` | argon2id over the operator's passphrase; already present for the admin password hash |
| S3 request signing | `hmac`, `sha2`, `base64` | AWS SigV4 written out rather than pulling the AWS SDK — see below. All three were already present for share-link tokens |
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

> **Offline / proxied builds.** Because the prebuilt-binary download uses
> `ort`'s bundled rustls roots, a TLS-intercepting proxy whose CA those roots
> don't trust makes a cold build fail with `invalid peer certificate:
> UnknownIssuer` (common in sandboxed CI / Claude Code on the web). Run
> `scripts/setup-onnxruntime.sh` to seed the prebuilt static library into the
> cache ort-sys checks before downloading (it fetches with `curl`, which uses
> the system CA store, and verifies the sha256 from ort-sys's `dist.txt`). The
> repo's SessionStart hook runs this automatically in web sessions.

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
| `aws-sdk-s3` | The offsite backup target makes exactly three requests — one `PUT`, one `GET ?list-type=2`, one `DELETE`. The SDK's dependency tree is larger than the rest of this binary, on a board whose release build is already dominated by ONNX Runtime and DuckDB. `SigV4` is four HMAC-SHA256 calls over fully specified strings, and is checked against vectors generated by **botocore** — one of which is AWS's own published example — rather than against a reading of the spec |
| `russh` / `ssh2` | The SFTP backup target drives OpenSSH's own `sftp` binary. An in-process SSH stack is a large dependency *and* a second place for key handling, host-key policy and cipher selection to be subtly wrong; `ssh2` additionally binds libssh2, which would mean a C dependency in the cross-compile image |

## Actual Dependency Count by Crate

Direct dependencies (excluding universal `serde` and `tracing`):

| Crate | Key direct dependencies |
|-------|-------------------------|
| `birdnet-core` | symphonia, rubato, realfft, notify, ort, ndarray, hound |
| `birdnet-db` | rusqlite |
| `birdnet-web` | axum, tower, tower-http, tokio, tokio-util, tokio-stream, sysinfo, reqwest, rustls, tokio-rustls, hyper, hyper-util, rcgen |
| `birdnet-integrations` | reqwest, tokio, lettre, rustls, ring, hmac, base64, argon2, tokio-util |
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
