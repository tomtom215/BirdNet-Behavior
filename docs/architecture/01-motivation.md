# Motivation

> Why rewrite BirdNET-Pi in Rust, and why not alternative languages.

## Table of Contents

- [Why Leave Python](#why-leave-python)
- [Why Rust](#why-rust)
- [Why Not Go](#why-not-go)
- [Design Philosophy](#design-philosophy)

---

## Why Leave Python

| Problem | Impact on Field Stations |
|---------|------------------------|
| Dependency rot | pip updates break installs; librosa/numpy/scipy version conflicts |
| Memory bloat | Python + librosa + TFLite interpreter: 300–600 MB RSS on a 1 GB Pi |
| GC pauses | Unpredictable latency during real-time analysis |
| No static typing enforcement | Runtime TypeErrors in production after weeks of uptime |
| Startup time | 5–15 s cold start importing numpy/scipy/librosa |
| Distribution | Requires virtualenv, pip, system Python matching — fragile on Debian upgrades |
| Process sprawl | BirdNET-Pi runs 6+ systemd services; complex inter-process coordination |

## Why Rust

| Advantage | BirdNET-Pi Benefit |
|-----------|-------------------|
| Zero-cost abstractions | Mel spectrogram computation without runtime overhead |
| Lean overhead | only the model's own footprint — no interpreter or venv resident on top |
| No GC | Deterministic latency for real-time audio pipeline |
| Single binary | `scp birdnet-behavior pi@station:` — done. No pip, no venv, no apt |
| Cross-compilation | Build for aarch64 on CI, deploy anywhere |
| Fearless concurrency | A dedicated capture/detection thread alongside async web serving — data-race-free by construction |
| Long-running stability | No memory leaks from reference cycles, no GIL contention |
| Pure Rust ecosystem | Audio pipeline (symphonia, rubato) has zero C dependencies |
| Embedded databases | rusqlite (SQLite) and DuckDB both have quality Rust bindings |

## Why Not Go

Go is a reasonable alternative, but Rust wins for this workload:

- **GC pauses** still exist (lower than Python but non-zero; matters for real-time audio)
- **Binary size** is larger — Go embeds its runtime, while Rust strips to near-C sizes with `strip = true`
- **Memory usage** is higher — goroutine stacks and GC overhead versus Rust's zero-cost abstractions
- **Audio / ML ecosystem** is weaker — there is no Go equivalent to `symphonia`, `rubato`, or `ort`
- **Embedded databases** — both DuckDB and SQLite have first-class Rust bindings that integrate cleanly into a single binary

## Design Philosophy

This project follows a **minimal dependency, pure Rust** philosophy:

1. **Prefer pure Rust crates** over C/C++ bindings wherever possible
2. **Hand-roll where simple** — don't add a crate for something achievable in <100 lines
3. **Zero system dependencies** for the audio pipeline (symphonia, rubato are pure Rust)
4. **Single binary deployment** — everything embedded, nothing to install
5. **No runtime overhead** — no garbage collector, no interpreter, no JIT
6. **Small, single-responsibility modules** — split a file when its purpose stops being one thing. Data-definition and orchestration files are the exception: `migration.rs` is ~3 100 lines by design, one entry per migration.
7. **Trait abstractions over concrete types** — every major boundary is a trait for testability

### Feature Parity

BirdNET-Pi users migrating from the Python version should find every
feature they rely on:

| Category | BirdNET-Pi (Python) | BirdNet-Behavior (Rust) |
|----------|--------------------|-----------------------|
| Detection pipeline | Yes | Yes |
| Web dashboard | Yes | HTMX + WebSocket + SSE live updates |
| Species pages with images | Yes | Wikipedia cache |
| Admin settings UI | Yes | Full settings editor |
| BirdWeather upload | Yes | With retry queue |
| Email alerts | Yes | SMTP via `lettre` |
| Apprise notifications | Yes | Yes |
| Backup management | Yes | List / download / delete UI |
| Activity heatmap | Yes | SVG hour × weekday |
| BirdNET-Pi import | — | Non-destructive migration wizard |
| Behavioral analytics | — | DuckDB OLAP layer |

---

[Back to Index](../RUST_ARCHITECTURE_PLAN.md) | [Next: Architecture →](02-architecture.md)
