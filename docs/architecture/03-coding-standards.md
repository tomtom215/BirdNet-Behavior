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

[workspace.lints.rust]
unsafe_code = "deny"
missing_debug_implementations = "warn"
```

**`unsafe` is denied workspace-wide.** No exceptions. This is a field-deployed
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

- **No async in library crates** (`birdnet-core`, `birdnet-db` are synchronous)
- **Tokio only in application code** (`birdnet-web` uses `tokio` with full features)
- Blocking operations via `tokio::task::spawn_blocking` for DB queries, file I/O, inference

This keeps library crates portable and testable without an async runtime.

## Modularity Rules

These rules are **hard requirements**, not suggestions:

### 1. No file over 500 lines

If a file approaches 500 lines, split it using Rust's module system:

```
routes/admin/mod.rs      → sub-module declarations + router assembly
routes/admin/settings.rs → settings form logic only
routes/admin/backup.rs   → backup list/download/delete only
routes/admin/system.rs   → system info + backup trigger only
routes/admin/logs.rs     → log streaming only
```

### 2. Single responsibility per module

Each `.rs` file has one clear purpose. Examples:
- `sqlite/settings.rs` — only key-value settings CRUD
- `sqlite/migrations.rs` — only schema migration logic
- `email.rs` — only SMTP email composition and delivery

### 3. Trait-based abstraction at every boundary

```rust
// ✅ Good: trait boundary enables testing and swapping implementations
pub trait Migrator {
    type Source;
    type Report;
    fn validate_source(&self, source: &Self::Source)
        -> Result<(SchemaInfo, SourceReport, MigrationReport), MigrateError>;
    fn migrate(&self, source: &Self::Source, target: &Connection)
        -> Result<MigrationReport, MigrateError>;
}

// ✅ Good: integration trait
pub trait NotificationSink: Send + Sync {
    fn notify(&self, detection: &Detection) -> impl Future<Output = Result<bool, Error>>;
}
```

### 4. Sub-modules for grouped functionality

```rust
// In birdnet-db/src/sqlite/mod.rs
pub mod connection;
pub mod migrations;
pub mod settings;
pub mod queries;          // further sub-modules inside

// In birdnet-db/src/sqlite/queries/mod.rs
pub mod detections;
pub mod species;
pub mod correlation;
pub mod analytics;
```

### 5. Re-export via `pub use` at crate root

Consumers use the crate's public API, not internal module paths:

```rust
// birdnet-db/src/lib.rs
pub use sqlite::connection::DbConnection;
pub use sqlite::settings::{get_or, set};
pub use sqlite::queries::detections::insert_detection;
```

## Testing Philosophy

- Unit tests within modules (`#[cfg(test)]` pattern)
- **Property-based testing** with `proptest` for data pipeline validation
- **Criterion.rs benchmarks** with HTML reports for performance-critical paths
- End-to-end tests against real WAV fixtures and real SQLite databases
- Coverage tracked via `cargo-tarpaulin`
- MSRV explicitly specified (1.88) and CI-enforced
- **Current test count**: ~516 passing across all crates and integration tests

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
2. **clippy** — `cargo clippy --workspace --all-targets -- -D warnings`
   (pedantic + nursery, zero warnings permitted)
3. **test** — `cargo test --workspace` (unit, integration, doc tests)
4. **doc** — `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`
5. **build** — debug build of the full workspace with and without the
   `analytics` feature
6. **msrv** — `cargo check --workspace` against the declared MSRV

Release builds for `aarch64`, `x86_64`, and `armv7` are produced by
`.github/workflows/release.yml` using `cargo-zigbuild`, and multi-arch
Docker images are assembled by `.github/workflows/docker.yml` on native
runners to avoid QEMU emulation.

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
