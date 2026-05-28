//! Optional compile-time embedding of the `behavioral` DuckDB community
//! extension binary.
//!
//! Locates the extension via, in order:
//!   1. the `BIRDNET_BUNDLED_EXTENSION_FILE` env var (release pipeline / CI);
//!   2. `crates/birdnet-behavioral/vendor/behavioral.duckdb_extension` (if a
//!      maintainer commits a vendored copy).
//!
//! When the binary is present and the `analytics` feature is enabled, the
//! bytes are staged into `OUT_DIR` and embedded via `include_bytes!`. The
//! runtime loader falls back to writing those bytes to a temp file and
//! `LOAD '<path>'` when the cached / community-registry paths fail —
//! making the extension genuinely load out of the box on installs without
//! network access at first run.
//!
//! When the binary is absent, the generated file declares
//! `EMBEDDED_EXTENSION: Option<&[u8]> = None` and the loader's behaviour is
//! unchanged from before this build script existed.

use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=BIRDNET_BUNDLED_EXTENSION_FILE");
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by cargo"));
    let generated = out_dir.join("embedded_extension.rs");

    // Only embed under the `analytics` feature (the loader that consumes it is
    // gated on `analytics`); without it there is no DuckDB connection to load
    // into, so embedding would just bloat the artifact.
    let analytics = env::var_os("CARGO_FEATURE_ANALYTICS").is_some();
    let source = locate_extension().filter(|_| analytics);
    if let Some(ref src) = source {
        println!("cargo:rerun-if-changed={}", src.display());
    }

    let body = source.map_or_else(
        || "pub(crate) const EMBEDDED_EXTENSION: Option<&[u8]> = None;\n".to_string(),
        |src| {
            // Stage into OUT_DIR so `include_bytes!` has a stable path and the
            // build is hermetic regardless of where the source lives.
            let staged = out_dir.join("behavioral.duckdb_extension");
            fs::copy(&src, &staged).unwrap_or_else(|e| {
                panic!("failed to stage bundled extension {}: {e}", src.display())
            });
            format!(
                "pub(crate) const EMBEDDED_EXTENSION: Option<&[u8]> = \
                 Some(include_bytes!(r\"{}\"));\n",
                staged.display()
            )
        },
    );

    fs::write(&generated, body).expect("write embedded_extension.rs");
}

fn locate_extension() -> Option<PathBuf> {
    if let Some(path) = env::var_os("BIRDNET_BUNDLED_EXTENSION_FILE")
        .map(PathBuf::from)
        .filter(|p| p.is_file())
    {
        return Some(path);
    }
    let vendored = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir set"))
        .join("vendor")
        .join("behavioral.duckdb_extension");
    vendored.is_file().then_some(vendored)
}
