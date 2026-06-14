//! Optional compile-time embedding of the `behavioral` DuckDB community
//! extension binary.
//!
//! Locates the extension via, in order:
//!   1. the `BIRDNET_BUNDLED_EXTENSION_FILE` env var (release pipeline / CI);
//!   2. `crates/birdnet-behavioral/vendor/behavioral-<platform>.duckdb_extension`
//!      matching the build *target* (so a cross-build for the Pi embeds the
//!      `linux_arm64` binary, never the host's `linux_amd64`);
//!   3. `crates/birdnet-behavioral/vendor/behavioral.duckdb_extension` — the
//!      legacy single-file fallback (kept for backward compatibility).
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
    // 1. Explicit override — the release pipeline / CI downloads the build
    //    matching the bundled DuckDB version and points this at it.
    if let Some(path) = env::var_os("BIRDNET_BUNDLED_EXTENSION_FILE")
        .map(PathBuf::from)
        .filter(|p| p.is_file())
    {
        return Some(path);
    }
    let vendor =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir set")).join("vendor");
    // 2. Platform-specific vendored copy. The embedded binary must match the
    //    *target* platform (DuckDB verifies it at LOAD time), so a cross-build
    //    for the Pi (aarch64) embeds `behavioral-linux_arm64.duckdb_extension`,
    //    not the x86_64 host's copy.
    if let Some(platform) = duckdb_target_platform() {
        let by_platform = vendor.join(format!("behavioral-{platform}.duckdb_extension"));
        if by_platform.is_file() {
            return Some(by_platform);
        }
    }
    // 3. Legacy single-file fallback (kept so an existing vendored copy keeps
    //    working without renaming).
    let legacy = vendor.join("behavioral.duckdb_extension");
    legacy.is_file().then_some(legacy)
}

/// Map the Cargo build target to a DuckDB extension platform string
/// (e.g. `linux_amd64`, `linux_arm64`, `osx_arm64`), matching the community
/// registry's per-platform paths. Returns `None` for targets the registry does
/// not name, in which case only the legacy single-file fallback applies.
fn duckdb_target_platform() -> Option<String> {
    let os = env::var("CARGO_CFG_TARGET_OS").ok()?;
    let arch = env::var("CARGO_CFG_TARGET_ARCH").ok()?;
    let platform = match (os.as_str(), arch.as_str()) {
        ("linux", "x86_64") => "linux_amd64",
        ("linux", "aarch64") => "linux_arm64",
        ("macos", "x86_64") => "osx_amd64",
        ("macos", "aarch64") => "osx_arm64",
        _ => return None,
    };
    Some(platform.to_owned())
}
