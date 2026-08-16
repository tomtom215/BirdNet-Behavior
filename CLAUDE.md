# BirdNet-Behavior Repository Reference

**A Rust rewrite of BirdNET-Pi with DuckDB behavioral analytics.**

BirdNet-Behavior is a real-time acoustic bird classification system targeting
Raspberry Pi (5, 4B, 400) and x86_64 Linux, built as a single Rust binary.
It integrates [duckdb-behavioral](https://github.com/tomtom215/duckdb-behavioral)
for bird activity analytics (sessionization, retention, funnel analysis, sequence matching).

## Lineage & Attribution

This project is derived from BirdNET-Pi (CC BY-NC-SA 4.0):
- **BirdNET**: K. Lisa Yang Center for Conservation Bioacoustics, Cornell University
- **BirdNET-Pi**: Patrick McGuire (mcguirepr89)
- **BirdNET-Pi fork**: Nachtzuster
- **BirdNET-Pi fork**: tomtom215

See `LICENSE` and `LICENSE-UPSTREAM` for full attribution and license terms.

## Architecture

Single Rust binary with 8 workspace crates:

| Crate | Purpose |
|-------|---------|
| `birdnet-core` | Audio capture, decode, resample, mel spectrogram, ML inference, detection pipeline, tmpfs, live spectrogram |
| `birdnet-db` | SQLite (OLTP) + DuckDB (OLAP), resilience, migrations |
| `birdnet-web` | axum web server, REST API, WebSocket, HTMX templates, audio player, admin |
| `birdnet-integrations` | BirdWeather, Apprise, MQTT (Home Assistant discovery), Wikipedia images, email, heartbeat, weekly reports, auto-update |
| `birdnet-behavioral` | DuckDB behavioral analytics for bird activity patterns |
| `birdnet-timeseries` | Time-series analytics (activity, diversity, trend, peak, gap, sessions) |
| `birdnet-migrate` | BirdNET-Pi migration: schema detection, validation, import |
| `birdnet-scheduler` | Solar calculations, recording window scheduling |

See `docs/RUST_ARCHITECTURE_PLAN.md` for the full phased implementation plan.

## Quick Reference

### Build & Test

```bash
# Build (debug)
cargo build

# Build (release, optimized for Pi deployment)
cargo build --release

# Run tests
cargo test

# Run with clippy lints
cargo clippy --workspace --all-targets

# Format check
cargo fmt --check --all
```

> **Cold build fails with `ort-sys ... invalid peer certificate: UnknownIssuer`?**
> You're behind a TLS-intercepting proxy that `ort`'s bundled rustls roots don't
> trust (sandboxed CI / Claude Code on the web). Run `scripts/setup-onnxruntime.sh`
> once to seed the ONNX Runtime download cache via `curl`; builds then work
> offline. Web sessions run this automatically via the SessionStart hook.

### Cross-compilation (for Raspberry Pi)

```bash
# Install target
rustup target add aarch64-unknown-linux-gnu

# Build with cross
cross build --release --target aarch64-unknown-linux-gnu
```

### Coding Conventions

- **No `anyhow`/`thiserror` in library crates** - hand-rolled error types
- **No async in the compute/storage library crates** (e.g. `birdnet-core`, `birdnet-db`) - they are synchronous. `birdnet-integrations` is the deliberate exception: an async *client* library for network I/O that constructs no runtime of its own
- **The tokio runtime is owned by application code** (`birdnet-web`, main binary) - library crates never start their own runtime
- **Blocking ops via `tokio::task::spawn_blocking`** for DB, file I/O, inference
- **`unsafe` is forbidden** workspace-wide (`unsafe_code = "forbid"`)
- **`missing_docs` enforced** workspace-wide
- **Clippy pedantic + nursery** enabled

### Testing Conventions

**A new gate must be observed failing against the code it was written for, and
the commit message must say how.**

Not "it should fail" — apply the old code, remove the fix, or mutate the
constant, and watch it go red. A test written after the fix and only ever seen
green proves nothing about what it would catch. This is cheap (one revert, one
`cargo test`) and it has repeatedly caught tests that were green for reasons
that had nothing to do with what they claimed to assert:

- a gate satisfied by a cache an earlier probe had populated
- a gate satisfied by test-execution ordering
- a gate asserting only the rejecting side of a boundary
- a preview query that agreed with its migration only by coincidence

When a gate covers a discrimination rather than a single behaviour, write the
counterpart too and check it stays green — otherwise a blanket alarm passes for
a discriminator.

Corollaries, each learned the same way:

- **Build the smallest thing that settles the question, and run it.** A scratch
  probe (`crates/*/tests/zz_*.rs`, or a `zz_probe_*` unit test, deleted before
  commit) beats any amount of reasoning about what the code does. Two of the
  facts in `--channel-report`'s own doc comments were wrong until one was run.
- **A test that passes tells you nothing until you know why it passes.**
- **Distrust confident prose in this repo's history, including your own.** The
  `load_icu()` comment asserted ICU was statically linked; it was not.
  `aligned_sum` was documented as summing; it averaged. Both misled the next
  reader.
- **`cmd 2>&1 | tail -N` masks the exit code.** A full workspace run has
  reported "exit 0" with two failures inside it. Grep for `^test result:` and
  `FAILED`, or check `PIPESTATUS`.
- **`~/.duckdb` contaminates ICU results.** One probe populates it and every
  ICU-related test then passes for free. `mv ~/.duckdb /tmp/duckdb-cache-backup`
  before trusting any of them.
- **`pull_request`-triggered gates never see un-PR'd branches.** Open a draft PR
  early, or run the gates locally; work has sat broken on a pushed branch for
  hours because nothing was watching it.

### Key Dependencies

| Purpose | Crate |
|---------|-------|
| Async runtime | `tokio` |
| Web framework | `axum` |
| SQLite | `rusqlite` (bundled) |
| Audio decode | `symphonia` |
| Resampling | `rubato` |
| ML inference | `ort` (ONNX Runtime) |
| File watching | `notify` |
| Logging | `tracing` |

### MSRV

Rust 1.95 (edition 2024)
