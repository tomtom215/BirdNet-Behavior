# Architecture

BirdNet-Behavior is a single binary built from eight Rust workspace crates, each with a clear responsibility:

```text
birdnet-behavior (single binary)
├── birdnet-core          Audio capture, decode, resample, mel spectrogram, ML inference
├── birdnet-db            SQLite (OLTP) + DuckDB (OLAP), migrations, resilience, backup
├── birdnet-web           axum web server, REST API, WebSocket, HTMX templates
├── birdnet-integrations  BirdWeather, Apprise, email, Wikipedia images, MQTT, auto-update
├── birdnet-behavioral    DuckDB behavioral analytics (feature-gated)
├── birdnet-timeseries    Activity, diversity, trend, peak, gap, session analytics
├── birdnet-migrate       BirdNET-Pi schema detection, validation, import
└── birdnet-scheduler     Solar calculations, recording-window scheduling
```

## Design principles

- **One static binary** — no Python, no runtime dependencies. Deploy by copying a file.
- **Synchronous, `unsafe`-free library crates** — `birdnet-core` and `birdnet-db` are blocking; async (Tokio) lives only in the application layer. Blocking work runs on `tokio::task::spawn_blocking`.
- **Hand-rolled error types** in libraries (no `anyhow`/`thiserror`), with `unsafe` denied workspace-wide and Clippy pedantic + nursery enforced.

## Deep-dive design documents

The full architecture is documented in the repository under [`docs/architecture/`](https://github.com/tomtom215/BirdNet-Behavior/tree/main/docs/architecture):

| Document | Contents |
|---|---|
| [01 — Motivation](https://github.com/tomtom215/BirdNet-Behavior/blob/main/docs/architecture/01-motivation.md) | Why Rust, not Python or Go |
| [02 — Architecture](https://github.com/tomtom215/BirdNet-Behavior/blob/main/docs/architecture/02-architecture.md) | Single-binary design and workspace layout |
| [03 — Coding standards](https://github.com/tomtom215/BirdNet-Behavior/blob/main/docs/architecture/03-coding-standards.md) | Linting, error handling, modularity, testing |
| [05 — Audio pipeline](https://github.com/tomtom215/BirdNet-Behavior/blob/main/docs/architecture/05-audio-pipeline.md) | Capture, decode, resample, segmentation |
| [06 — ML inference](https://github.com/tomtom215/BirdNet-Behavior/blob/main/docs/architecture/06-ml-inference.md) | Mel spectrogram and the BirdNET+ model |
| [07 — Database](https://github.com/tomtom215/BirdNet-Behavior/blob/main/docs/architecture/07-database.md) | SQLite + DuckDB, migrations, resilience |
| [10 — Deployment](https://github.com/tomtom215/BirdNet-Behavior/blob/main/docs/architecture/10-deployment.md) | Cross-compilation, CI/CD, systemd |

The index lives at [`docs/RUST_ARCHITECTURE_PLAN.md`](https://github.com/tomtom215/BirdNet-Behavior/blob/main/docs/RUST_ARCHITECTURE_PLAN.md).
