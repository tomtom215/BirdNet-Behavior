# birdnet-db

Database layer: SQLite (operational) with WAL resilience, backup, and integrity.

Part of [BirdNet-Behavior](https://github.com/tomtom215/BirdNet-Behavior), a Rust
rewrite of BirdNET-Pi with DuckDB behavioral analytics: one binary for real-time
acoustic bird classification on Raspberry Pi and x86_64 Linux.

**This crate is an implementation detail of that binary.** It is published so
that `cargo install birdnet-behavior` can build from crates.io. Its public API
carries no stability promise and its version moves in lockstep with the binary's
(see [STABILITY.md](https://github.com/tomtom215/BirdNet-Behavior/blob/main/STABILITY.md));
depend on it only if you are prepared to follow every release.

Licensed under
[CC BY-NC-SA 4.0](https://creativecommons.org/licenses/by-nc-sa/4.0/): non-commercial
use only. The full terms, the lineage and the third-party notices are in the
[repository](https://github.com/tomtom215/BirdNet-Behavior).
