# Coding Standards & Conventions

> Derived from tomtom215's established Rust patterns across duckdb-behavioral,
> quack-rs, and mallardmetrics.

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

- **Hand-rolled error types** -- no `anyhow` or `thiserror` in library crates
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

## Testing Philosophy

- Unit tests within modules (`#[cfg(test)]` pattern)
- **Property-based testing** with `proptest` for data pipeline validation
- **Criterion.rs benchmarks** with HTML reports for performance-critical paths
- E2E tests against real systems (DuckDB CLI, actual WAV files)
- **Mutation testing** via `cargo-mutants` (target: >85% kill rate)
- Coverage tracked via `cargo-tarpaulin` + Codecov
- MSRV explicitly specified (1.85) and CI-enforced

## CI/CD (GitHub Actions)

Following duckdb-behavioral's proven workflow pattern:

1. **Quality**: `fmt` → `clippy` → `check` → `doc` (fail fast)
2. **Testing**: `nextest` on Ubuntu + macOS, MSRV verification
3. **Security**: `cargo-deny` supply chain audit, CodeQL static analysis
4. **Compatibility**: SemVer check against main branch
5. **Coverage**: `cargo-tarpaulin` → Codecov
6. **Release**: Multi-platform builds with provenance attestation

Action versions pinned by **commit SHA** (not tags) for reproducibility.
Concurrency cancellation for redundant PR runs.

## Code Style

- Prefer `impl` blocks close to their type definitions
- Use `Self` over repeating the type name
- Prefer iterators and combinators over manual loops where readable
- Keep functions short -- if a function exceeds ~40 lines, consider splitting
- No `unwrap()` in library code; `expect()` only with descriptive messages in app code
- Prefer returning `Result` over panicking

---

[← Architecture](02-architecture.md) | [Back to Index](../RUST_ARCHITECTURE_PLAN.md) | [Next: Dependencies →](04-dependencies.md)
