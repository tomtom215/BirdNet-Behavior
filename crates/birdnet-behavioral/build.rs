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
//!
//! # Why this script parses the extension footer
//!
//! A DuckDB extension is version-locked: the engine refuses to `LOAD` a build
//! targeting any other DuckDB version, and `allow_extensions_metadata_mismatch`
//! does not bypass that check. Embedding is therefore only useful if the bytes
//! target *exactly* the engine `libduckdb-sys` links in.
//!
//! Nothing used to check that. `Dockerfile` pinned the extension to DuckDB
//! `v1.5.3` while the workspace bundled `v1.5.5`; the download succeeded, the
//! wrong bytes were embedded silently, and the only symptom was that offline
//! `LOAD` failed on stations with no network — the exact deployments the
//! embedding exists to serve. This script now refuses to embed bytes it cannot
//! parse, and records what they target so a test can assert the match against
//! the linked engine (which is only knowable at run time — cargo exposes no
//! `DEP_DUCKDB_*` version to this script; verified by probing its environment).
//!
//! # Footer layout
//!
//! Measured against the published `behavioral` builds for DuckDB v1.5.3 and
//! v1.5.5 (`community-extensions.duckdb.org`), not taken from documentation:
//!
//! ```text
//! last 512 bytes:
//!   [  0: 96]  reserved (zero)
//!   [ 96:128]  ABI type              "C_STRUCT_UNSTABLE"
//!   [128:160]  extension version     "v0.9.1"
//!   [160:192]  DuckDB version        "v1.5.5"
//!   [192:224]  platform              "linux_amd64"
//!   [224:256]  metadata format ver.  "4"
//!   [256:512]  signature block
//! ```
//!
//! Each field is NUL-padded to 32 bytes.

use std::{env, fs, path::PathBuf};

/// Total length of the metadata footer appended to every DuckDB extension.
const FOOTER_LEN: usize = 512;
/// Width of one NUL-padded metadata field.
const FIELD_LEN: usize = 32;
/// Field index of the extension's own version (e.g. `v0.9.1`).
const IDX_EXTENSION_VERSION: usize = 4;
/// Field index of the DuckDB version the extension was built for.
const IDX_DUCKDB_VERSION: usize = 5;
/// Field index of the target platform (e.g. `linux_amd64`).
const IDX_PLATFORM: usize = 6;
/// Field index of the footer's own format version.
const IDX_METADATA_VERSION: usize = 7;
/// Leading bytes of a gzip stream, used only to give a precise error when the
/// `.gz` the CDN serves is handed over without being decompressed.
const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];
/// The only footer format this script knows how to read.
///
/// If DuckDB bumps this, the build fails loudly rather than embedding bytes
/// whose layout we would be guessing at. Re-measure the layout, then bump.
const SUPPORTED_METADATA_VERSION: &str = "4";

/// What an extension binary declares about itself.
struct ExtensionMetadata {
    /// DuckDB version the extension was compiled against, e.g. `v1.5.5`.
    duckdb_version: String,
    /// The extension's own version, e.g. `v0.9.1`.
    extension_version: String,
    /// Target platform triple, e.g. `linux_amd64`.
    platform: String,
}

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
        || {
            concat!(
                "pub(crate) const EMBEDDED_EXTENSION: Option<&[u8]> = None;\n",
                "pub(crate) const EMBEDDED_EXTENSION_DUCKDB_VERSION: Option<&str> = None;\n",
                "pub(crate) const EMBEDDED_EXTENSION_VERSION: Option<&str> = None;\n",
                "pub(crate) const EMBEDDED_EXTENSION_PLATFORM: Option<&str> = None;\n",
            )
            .to_string()
        },
        |src| {
            let bytes = fs::read(&src).unwrap_or_else(|e| {
                panic!("failed to read bundled extension {}: {e}", src.display())
            });
            // Refuse to embed anything we cannot identify. Shipping unverified
            // bytes is what produced the v1.5.3/v1.5.5 defect.
            let meta = parse_metadata(&bytes).unwrap_or_else(|e| {
                panic!(
                    "{} is not a readable DuckDB extension: {e}\n\
                     Refusing to embed it. Check that BIRDNET_BUNDLED_EXTENSION_FILE points at \
                     an un-gzipped `behavioral.duckdb_extension` downloaded from \
                     community-extensions.duckdb.org for the DuckDB version this workspace \
                     bundles (see the `duckdb` pin in the root Cargo.toml).",
                    src.display()
                )
            });
            println!(
                "cargo:warning=embedding behavioral {} for DuckDB {} ({})",
                meta.extension_version, meta.duckdb_version, meta.platform
            );

            // Stage into OUT_DIR so `include_bytes!` has a stable path and the
            // build is hermetic regardless of where the source lives.
            let staged = out_dir.join("behavioral.duckdb_extension");
            fs::copy(&src, &staged).unwrap_or_else(|e| {
                panic!("failed to stage bundled extension {}: {e}", src.display())
            });
            format!(
                "pub(crate) const EMBEDDED_EXTENSION: Option<&[u8]> = \
                 Some(include_bytes!(r\"{}\"));\n\
                 pub(crate) const EMBEDDED_EXTENSION_DUCKDB_VERSION: Option<&str> = Some(\"{}\");\n\
                 pub(crate) const EMBEDDED_EXTENSION_VERSION: Option<&str> = Some(\"{}\");\n\
                 pub(crate) const EMBEDDED_EXTENSION_PLATFORM: Option<&str> = Some(\"{}\");\n",
                staged.display(),
                meta.duckdb_version,
                meta.extension_version,
                meta.platform,
            )
        },
    );

    fs::write(&generated, body).expect("write embedded_extension.rs");
}

/// Read one NUL-padded 32-byte field out of the footer.
///
/// Returns `Err` when the field is not valid UTF-8, which for these fields
/// means the file is not the format we think it is.
fn footer_field(footer: &[u8], idx: usize) -> Result<String, String> {
    let start = idx * FIELD_LEN;
    let raw = &footer[start..start + FIELD_LEN];
    let end = raw.iter().position(|&b| b == 0).unwrap_or(FIELD_LEN);
    std::str::from_utf8(&raw[..end])
        .map(str::to_owned)
        .map_err(|e| format!("metadata field {idx} is not UTF-8: {e}"))
}

/// Parse the trailing metadata footer of a DuckDB extension binary.
fn parse_metadata(bytes: &[u8]) -> Result<ExtensionMetadata, String> {
    // The community CDN serves `.gz`, so handing the compressed file straight
    // through is the easy mistake. A compressed extension is ~130 kB, well past
    // the footer-length check, and would otherwise surface as an opaque UTF-8
    // error on a field index nobody should have to think about.
    if bytes.starts_with(&GZIP_MAGIC) {
        return Err(
            "file is gzip-compressed (starts with the gzip magic bytes); decompress it first — \
             community-extensions.duckdb.org serves `behavioral.duckdb_extension.gz`"
                .to_string(),
        );
    }
    if bytes.len() < FOOTER_LEN {
        return Err(format!(
            "file is {} bytes, shorter than the {FOOTER_LEN}-byte metadata footer every DuckDB \
             extension carries",
            bytes.len()
        ));
    }
    let footer = &bytes[bytes.len() - FOOTER_LEN..];

    let metadata_version = footer_field(footer, IDX_METADATA_VERSION)?;
    if metadata_version != SUPPORTED_METADATA_VERSION {
        return Err(format!(
            "footer declares metadata format version {metadata_version:?}, but this build script \
             only knows how to read version {SUPPORTED_METADATA_VERSION:?}. Re-measure the footer \
             layout before bumping SUPPORTED_METADATA_VERSION"
        ));
    }

    let duckdb_version = footer_field(footer, IDX_DUCKDB_VERSION)?;
    let extension_version = footer_field(footer, IDX_EXTENSION_VERSION)?;
    let platform = footer_field(footer, IDX_PLATFORM)?;

    if duckdb_version.is_empty() {
        return Err("footer carries no DuckDB version".to_string());
    }
    if platform.is_empty() {
        return Err("footer carries no target platform".to_string());
    }

    Ok(ExtensionMetadata {
        duckdb_version,
        extension_version,
        platform,
    })
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
