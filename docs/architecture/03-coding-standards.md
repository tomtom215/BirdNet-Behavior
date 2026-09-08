# Coding Standards & Conventions

> Conventions applied uniformly across every workspace crate.

## Table of Contents

- [Release Profile](#release-profile)
- [Linting](#linting)
- [Error Handling](#error-handling)
- [Async Convention](#async-convention)
- [Modularity Rules](#modularity-rules)
- [Testing Philosophy](#testing-philosophy)
- [CI/CD](#cicd-github-actions)
- [Code Style](#code-style)

---

## Release Profile

```toml
[profile.release]
opt-level = 3
lto = true
codegen-units = 1
panic = "abort"
strip = true
```

This produces the smallest, fastest binary. On Pi 4/5, `lto = true` with
`codegen-units = 1` enables whole-program optimization that matters for
constrained hardware.

## Linting

```toml
[workspace.lints.clippy]
all = { level = "warn", priority = -1 }
pedantic = { level = "warn", priority = -1 }
nursery = { level = "warn", priority = -1 }
cargo = { level = "warn", priority = -1 }
# Pragmatic allowances
module_name_repetitions = "allow"
must_use_candidate = "allow"
multiple_crate_versions = "allow"
duration_suboptimal_units = "allow"

[workspace.lints.rust]
unsafe_code = "forbid"
missing_debug_implementations = "warn"
missing_docs = "warn"
```

**`unsafe` is forbidden workspace-wide** (`forbid`, not `deny`, so a stray `#[allow]` can't reintroduce it). No exceptions. This is a field-deployed
system that must run unattended for months.

## Error Handling

- **Hand-rolled error types** — no `anyhow` or `thiserror` in library crates
- Custom enum-based errors with `Display` and `Error` trait implementations
- `Result<T, E>` throughout; never panic across FFI or async boundaries
- Application code (`main.rs`) may use `Box<dyn Error>` for convenience

Pattern used throughout the codebase:

```rust
#[derive(Debug)]
pub enum DecodeError {
    Io(std::io::Error),
    Format(String),
    NoTracks,
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::Format(msg) => write!(f, "format error: {msg}"),
            Self::NoTracks => write!(f, "no audio tracks found"),
        }
    }
}

impl std::error::Error for DecodeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}
```

## Async Convention

- **No async in the compute/storage library crates** (`birdnet-core`, `birdnet-db` are synchronous)
- **Tokio only in application code** (`birdnet-web` uses `tokio` with full features) and in `birdnet-integrations`, the deliberate exception: an async client library for network I/O that never constructs a runtime of its own
- Blocking operations via `tokio::task::spawn_blocking` for DB queries, file I/O, inference

This keeps library crates portable and testable without an async runtime.

## Modularity Rules

These rules are **hard requirements**, not suggestions:

### 1. Prefer files under 500 lines

500 lines is the point at which a file should be justified rather than a
hard cap: a data table with one entry per row (`migration.rs`, ~3 200
lines) or a single orchestration loop legitimately exceeds it, and over
a hundred files in the workspace do. Behaviour spread across 500 lines
usually should not. When a file grows past it for no such reason, split
it using Rust's module system:

```
routes/admin/mod.rs      → sub-module declarations + router assembly
routes/admin/settings/   → settings form logic only (mod.rs, handler.rs, form.rs, render/)
routes/admin/backup.rs   → backup list/download/delete only
routes/admin/system.rs   → system info + backup trigger only
routes/admin/logs.rs     → log streaming only
```

### 2. Single responsibility per module

Each `.rs` file has one clear purpose. Examples:
- `settings.rs` — only key-value settings CRUD
- `migration.rs` — only the versioned schema-migration chain
- `email/` (`mod.rs`, `smtp.rs`, `templates.rs`, `types.rs`) — only SMTP email composition and delivery

### 3. Trait-based abstraction at every boundary

```rust
// ✅ Good: one narrow trait per responsibility, each with one job
pub trait SchemaDetector: Send + Sync {
    fn detect(&self, path: &Path) -> Result<DetectedSchema, MigrateError>;
}

pub trait Validator: Send + Sync {
    fn validate_source(&self, source_path: &Path) -> Result<ValidationReport, MigrateError>;
    fn validate_destination(&self, source_path: &Path, dest_path: &Path)
        -> Result<ValidationReport, MigrateError>;
}

pub trait Migrator: Send + Sync {
    fn migrate(&self, source_path: &Path, dest_path: &Path, progress: &ProgressHandle)
        -> Result<MigrationSummary, MigrateError>;
}

// ✅ Good: a clock boundary (birdnet-scheduler/src/traits.rs) so schedule
// logic is tested against a fake clock rather than the wall clock
pub trait TimeSource: Send + Sync {
    /// Returns minutes since midnight (0–1439) in local time.
    fn minutes_since_midnight(&self) -> u32;
    /// Returns the current day-of-year (1–366).
    fn day_of_year(&self) -> u32;
}
```

### 4. Sub-modules for grouped functionality

```rust
// In birdnet-db/src/sqlite/mod.rs
pub mod connection;
pub mod queries;          // further sub-modules inside
pub mod types;

// Settings and the migration chain sit at the crate root, not under sqlite/:
// birdnet-db/src/settings.rs, birdnet-db/src/migration.rs

// In birdnet-db/src/sqlite/queries/mod.rs
pub mod analytics;
pub mod correlation;
pub mod detection_reviews;
pub mod detections;       // itself split into read.rs / write.rs / …
pub mod effort;
pub mod heatmap;
pub mod images;
pub mod imports;
pub mod maintenance;
pub mod quarantine;
pub mod species;
```

### 5. Re-export via `pub use` at crate root

Consumers use the crate's public API, not internal module paths:

```rust
// birdnet-db/src/sqlite/mod.rs — flat re-exports so call sites stay
// `birdnet_db::sqlite::foo` regardless of which sub-module owns `foo`
pub use connection::{DbError, open_connection, open_or_create, open_readonly, quick_check};
pub use queries::correlation::{FollowOn, SpeciesPair};
```

## Testing Philosophy

- Unit tests within modules (`#[cfg(test)]` pattern)
- **Property-based testing** with `proptest` for data pipeline validation
- **Criterion.rs benchmarks** with HTML reports for performance-critical paths
- End-to-end tests against real WAV fixtures and real SQLite databases
- Coverage tracked via `cargo-llvm-cov` (`.github/workflows/coverage.yml`)
- MSRV explicitly specified (1.95) and CI-enforced
- **Current test count**: 3 694 `#[test]` / `#[tokio::test]` attributes across
  the workspace at commit `dd10fe7` — re-derive with
  `grep -rn --include='*.rs' -E '^\s*#\[(test|tokio::test)' src crates tests | wc -l`
  rather than trusting this figure

### Raw String Literals in HTML/SVG

When format strings contain HTML attributes with hex colors (e.g., `fill="#0f172a">`),
the `"#` sequence terminates `r#"..."#` raw strings. Always use `r##"..."##`:

```rust
// ❌ WRONG: "# in fill="#0f172a"> terminates the raw string
write!(f, r#"<rect fill="#0f172a" rx="8"/>"#)?;

// ✅ CORRECT: r##"..."## is not terminated by "#
write!(f, r##"<rect fill="#0f172a" rx="8"/>"##)?;
```

## CI/CD (GitHub Actions)

The `.github/workflows/ci.yml` pipeline enforces quality gates on every
push and pull request:

1. **fmt** — `cargo fmt --check --all` (zero diff required)
2. **clippy** — run twice: `cargo clippy --workspace --all-targets -- -D
   warnings`, then again with `--all-features` (pedantic + nursery, zero
   warnings permitted)
3. **test** — five invocations: `--workspace --lib --bins`, `--workspace
   --tests`, `--workspace --doc`, `--workspace --all-features`, and
   `-p birdnet-behavioral --features analytics
   embedded_extension_loads_when_bundled`
4. **inference** — the end-to-end suites run against the real model
5. **doc** — `cargo doc --workspace --no-deps --document-private-items
   --all-features` with warnings denied
6. **build** — debug build of the full workspace with and without the
   `analytics` feature
7. **msrv** — `cargo check --workspace --all-features` against the declared MSRV
8. **cross-aarch64** — `cargo check --workspace --all-features --target
   aarch64-unknown-linux-gnu`
9. **test-aarch64** — on a native `ubuntu-24.04-arm` runner, `cargo test -p
   birdnet-core -p birdnet-scheduler -p birdnet-timeseries --lib --tests`; the
   job's comment says which crates are excluded and why

Release builds for `aarch64`, `x86_64` and `aarch64-apple-darwin` are
produced by `.github/workflows/release.yml`; the two Linux targets use
Ubuntu 24.04's native GCC 13 cross toolchain and macOS builds natively on
`macos-14`, and multi-arch Docker images are assembled by
`.github/workflows/docker.yml` on native runners to avoid QEMU
emulation.

## Code Style

- Prefer `impl` blocks close to their type definitions
- Use `Self` over repeating the type name
- Prefer iterators and combinators over manual loops where readable
- Keep functions short — if a function exceeds ~40 lines, consider splitting
- No `unwrap()` in library code; `expect()` only with descriptive messages in app code
- Prefer returning `Result` over panicking
- Use `tracing::{debug, info, warn, error}` instead of `println!` / `eprintln!`
- Structured logging: `tracing::info!(species = %name, confidence = conf, "detection");`

---

[← Architecture](02-architecture.md) | [Back to Index](../RUST_ARCHITECTURE_PLAN.md) | [Next: Dependencies →](04-dependencies.md)
