# Motivation

> Why rewrite BirdNET-Pi in Rust, and why not alternative languages.

## Why Leave Python

| Problem | Impact on Field Stations |
|---------|------------------------|
| Dependency rot | pip updates break installs; librosa/numpy/scipy version conflicts |
| Memory bloat | Python + librosa + TFLite interpreter: 300-600 MB RSS on a 1 GB Pi |
| GC pauses | Unpredictable latency during real-time analysis |
| No static typing enforcement | Runtime TypeErrors in production after weeks of uptime |
| Startup time | 5-15s cold start importing numpy/scipy/librosa |
| Distribution | Requires virtualenv, pip, system Python matching -- fragile on Debian upgrades |

## Why Rust

| Advantage | BirdNET-Pi Benefit |
|-----------|-------------------|
| Zero-cost abstractions | Mel spectrogram computation without runtime overhead |
| Predictable memory | 20-50 MB RSS for entire station binary |
| No GC | Deterministic latency for real-time audio pipeline |
| Single binary | `scp birdnet-pi pi@station:` -- done. No pip, no venv, no apt |
| Cross-compilation | Build for aarch64 on CI, deploy anywhere |
| Fearless concurrency | Safe parallel audio processing and async web serving |
| Long-running stability | No memory leaks from reference cycles, no GIL contention |
| Pure Rust ecosystem | Audio pipeline (symphonia, rubato) has zero C dependencies |

## Why Not Go

Go is a reasonable alternative (and tomtom215 has production Go code: `lyrebirdaudio-go`
with Erlang-style supervision trees, Prometheus metrics, and 87% code coverage). However,
Rust wins for BirdNET-Pi:

- **GC pauses** still exist (lower than Python but non-zero; matters for real-time audio)
- **Binary size** is larger (Go embeds runtime; Rust strips to near-C sizes with `strip = true`)
- **Memory usage** is higher (goroutine stacks, GC overhead vs Rust's zero-cost abstractions)
- **Ecosystem for audio/ML** is weaker (no equivalent to symphonia, rubato, ort)
- **DuckDB ecosystem** is entirely Rust-based in tomtom215's repos (duckdb-behavioral, quack-rs, mallardmetrics)
- **Language selection pattern**: tomtom215 uses Go for infrastructure/systems tools (audio streaming, device management) and Rust for performance-critical work (analytics engines, FFI, web backends with embedded databases) -- BirdNET-Pi is firmly in the latter category

## Design Philosophy

This project follows a **minimal dependency, pure Rust** philosophy:

1. **Prefer pure Rust crates** over C/C++ bindings wherever possible
2. **Hand-roll where simple** -- don't add a crate for something achievable in <100 lines
3. **Zero system dependencies** for the audio pipeline (symphonia, rubato are pure Rust)
4. **Single binary deployment** -- everything embedded, nothing to install
5. **No runtime overhead** -- no garbage collector, no interpreter, no JIT

---

[Back to Index](../RUST_ARCHITECTURE_PLAN.md) | [Next: Architecture →](02-architecture.md)
