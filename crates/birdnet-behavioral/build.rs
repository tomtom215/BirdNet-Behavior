//! Optional compile-time embedding of the DuckDB extension binaries this
//! crate needs: the `behavioral` community extension, and `icu`.
//!
//! Each is located via, in order:
//!   1. an env var pointing at the file (release pipeline / CI);
//!   2. a vendored copy under `crates/birdnet-behavioral/vendor/`.
//!
//! | Extension    | Env var                          | Vendored path                     |
//! |--------------|----------------------------------|-----------------------------------|
//! | `behavioral` | `BIRDNET_BUNDLED_EXTENSION_FILE` | `vendor/behavioral.duckdb_extension` |
//! | `icu`        | `BIRDNET_BUNDLED_ICU_FILE`       | `vendor/icu.duckdb_extension`     |
//!
//! When a binary is present and the `analytics` feature is enabled, the
//! bytes are staged into `OUT_DIR` and embedded via `include_bytes!`. The
//! runtime loader writes those bytes to a temp file and `LOAD '<path>'`s them
//! — making the extension genuinely load out of the box on installs without
//! network access at first run.
//!
//! When a binary is absent, the generated file declares that extension's
//! `Option<&[u8]>` as `None` and the loader falls back to whatever DuckDB can
//! find or fetch on its own.
//!
//! # Why `icu` is embedded and not just autoloaded
//!
//! `icu` is **not** statically linked into the `libduckdb` that `duckdb-rs`
//! bundles — measured, not assumed: with autoload and autoinstall disabled and
//! no local extension cache, `duckdb_extensions()` reports `icu` as
//! `installed=false, NOT_INSTALLED` (while `core_functions`, by contrast,
//! reports `STATICALLY_LINKED`).
//!
//! Everything that resolves the *current local date* lives in it —
//! `CURRENT_DATE`, `today()`, the `TimeZone` setting, and even
//! `CAST(now() AS DATE)`, which fails with "Unimplemented type for cast
//! (TIMESTAMP WITH TIME ZONE -> DATE)" without it. There is no ICU-free
//! spelling of "today" to fall back to, so every date-ranged dashboard query
//! depends on this extension being present.
//!
//! Left to itself DuckDB autoinstalls `icu` into `$HOME/.duckdb` on first use.
//! That is what broke v0.13.1 in the field: the shipped systemd unit sets
//! `ProtectHome=read-only`, the install failed with
//! `Failed to create directory "/home/pi/.duckdb": Read-only file system`, and
//! every analytics query failed from then on — for days, with the health
//! endpoint green. Embedding the bytes removes the network *and* the writable
//! `$HOME` from the path entirely.
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
//! v1.5.5 (`community-extensions.duckdb.org`) and the published `icu` build for
//! v1.5.5 (`extensions.duckdb.org`), not taken from documentation:
//!
//! ```text
//! last 512 bytes:
//!   [  0: 96]  reserved (zero)
//!   [ 96:128]  ABI type              "C_STRUCT_UNSTABLE"  (icu: "CPP")
//!   [128:160]  extension version     "v0.9.1"             (icu: "v1.5.5")
//!   [160:192]  DuckDB version        "v1.5.5"
//!   [192:224]  platform              "linux_amd64"
//!   [224:256]  metadata format ver.  "4"
//!   [256:512]  signature block
//! ```
//!
//! Each field is NUL-padded to 32 bytes. The ABI type differs between a
//! community C-API extension and a core C++ one and is deliberately not
//! checked here: `LOAD` enforces it, and the two fields this script acts on —
//! DuckDB version and platform — are the ones a build can get wrong silently.

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

/// One extension this crate may embed.
struct EmbedSpec {
    /// DuckDB's name for it, used in diagnostics and in the download hint.
    name: &'static str,
    /// Prefix of the generated Rust constants, e.g. `EMBEDDED_ICU` yields
    /// `EMBEDDED_ICU`, `EMBEDDED_ICU_DUCKDB_VERSION`, `EMBEDDED_ICU_VERSION`
    /// and `EMBEDDED_ICU_PLATFORM`.
    const_prefix: &'static str,
    /// Env var the release pipeline sets to point at an un-gzipped binary.
    env_var: &'static str,
    /// File name, both for the `vendor/` fallback and for the `OUT_DIR` copy.
    file_name: &'static str,
    /// Registry that publishes it, quoted back in the "refusing to embed" hint.
    registry: &'static str,
}

/// The extensions considered for embedding, in generated-constant order.
const EMBEDS: [EmbedSpec; 2] = [
    EmbedSpec {
        name: "behavioral",
        const_prefix: "EMBEDDED_EXTENSION",
        env_var: "BIRDNET_BUNDLED_EXTENSION_FILE",
        file_name: "behavioral.duckdb_extension",
        registry: "community-extensions.duckdb.org",
    },
    EmbedSpec {
        name: "icu",
        const_prefix: "EMBEDDED_ICU",
        env_var: "BIRDNET_BUNDLED_ICU_FILE",
        file_name: "icu.duckdb_extension",
        registry: "extensions.duckdb.org",
    },
];

fn main() {
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by cargo"));
    let generated = out_dir.join("embedded_extension.rs");

    // Only embed under the `analytics` feature (the loader that consumes it is
    // gated on `analytics`); without it there is no DuckDB connection to load
    // into, so embedding would just bloat the artifact.
    let analytics = env::var_os("CARGO_FEATURE_ANALYTICS").is_some();

    let mut body = String::new();
    for spec in &EMBEDS {
        println!("cargo:rerun-if-env-changed={}", spec.env_var);
        let source = locate_extension(spec).filter(|_| analytics);
        if let Some(ref src) = source {
            println!("cargo:rerun-if-changed={}", src.display());
        }
        body.push_str(&generate(spec, source.as_deref(), &out_dir));
    }

    fs::write(&generated, body).expect("write embedded_extension.rs");
}

/// Emit the four constants for one extension, embedding `source` when present.
fn generate(
    spec: &EmbedSpec,
    source: Option<&std::path::Path>,
    out_dir: &std::path::Path,
) -> String {
    let prefix = spec.const_prefix;
    let Some(src) = source else {
        return format!(
            "pub(crate) const {prefix}: Option<&[u8]> = None;\n\
             pub(crate) const {prefix}_DUCKDB_VERSION: Option<&str> = None;\n\
             pub(crate) const {prefix}_VERSION: Option<&str> = None;\n\
             pub(crate) const {prefix}_PLATFORM: Option<&str> = None;\n",
        );
    };

    let bytes = fs::read(src)
        .unwrap_or_else(|e| panic!("failed to read bundled extension {}: {e}", src.display()));
    // Refuse to embed anything we cannot identify. Shipping unverified bytes is
    // what produced the v1.5.3/v1.5.5 defect.
    let meta = parse_metadata(&bytes).unwrap_or_else(|e| {
        panic!(
            "{} is not a readable DuckDB extension: {e}\n\
             Refusing to embed it. Check that {} points at an un-gzipped `{}` downloaded from \
             {} for the DuckDB version this workspace bundles (see the `duckdb` pin in the root \
             Cargo.toml).",
            src.display(),
            spec.env_var,
            spec.file_name,
            spec.registry,
        )
    });
    // Cargo does not tell a build script which DuckDB version `libduckdb-sys`
    // will link (verified by probing its environment), so that check has to
    // wait for run time. It *does* tell us the target triple, so the platform
    // half can be caught here — before 20 MB of unloadable ICU is baked into an
    // artifact whose only symptom would be silence on the Pi.
    if let Some(target) = duckdb_platform()
        && target != meta.platform
    {
        panic!(
            "{} targets platform {:?}, but this build targets {:?}.\n\
             Refusing to embed it: DuckDB will not LOAD an extension built for another platform, \
             so the bytes would be dead weight and the offline load would fail on the device. \
             Point {} at the {} build for {}.",
            src.display(),
            meta.platform,
            target,
            spec.env_var,
            spec.name,
            target,
        );
    }
    println!(
        "cargo:warning=embedding {} {} for DuckDB {} ({})",
        spec.name, meta.extension_version, meta.duckdb_version, meta.platform
    );

    // Stage into OUT_DIR so `include_bytes!` has a stable path and the build is
    // hermetic regardless of where the source lives.
    let staged = out_dir.join(spec.file_name);
    fs::copy(src, &staged)
        .unwrap_or_else(|e| panic!("failed to stage bundled extension {}: {e}", src.display()));
    format!(
        "pub(crate) const {prefix}: Option<&[u8]> = Some(include_bytes!(r\"{}\"));\n\
         pub(crate) const {prefix}_DUCKDB_VERSION: Option<&str> = Some(\"{}\");\n\
         pub(crate) const {prefix}_VERSION: Option<&str> = Some(\"{}\");\n\
         pub(crate) const {prefix}_PLATFORM: Option<&str> = Some(\"{}\");\n",
        staged.display(),
        meta.duckdb_version,
        meta.extension_version,
        meta.platform,
    )
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

/// `DuckDB`'s platform string for the target this build is producing.
///
/// `None` for any target this mapping does not cover, which is treated as "we
/// could not tell" rather than a disagreement — the same policy the runtime
/// mismatch check applies. Inventing a mismatch from missing information would
/// fail perfectly good builds, and the runtime check still backstops it.
fn duckdb_platform() -> Option<String> {
    let arch = env::var("CARGO_CFG_TARGET_ARCH").ok()?;
    let os = env::var("CARGO_CFG_TARGET_OS").ok()?;
    let base = match (os.as_str(), arch.as_str()) {
        ("linux", "x86_64") => "linux_amd64",
        ("linux", "aarch64") => "linux_arm64",
        ("macos", "aarch64") => "osx_arm64",
        ("macos", "x86_64") => "osx_amd64",
        ("windows", "x86_64") => "windows_amd64",
        _ => return None,
    };
    // DuckDB publishes musl builds under their own platform names.
    if os == "linux" && env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("musl") {
        return Some(format!("{base}_musl"));
    }
    Some(base.to_owned())
}

fn locate_extension(spec: &EmbedSpec) -> Option<PathBuf> {
    if let Some(path) = env::var_os(spec.env_var)
        .map(PathBuf::from)
        .filter(|p| p.is_file())
    {
        return Some(path);
    }
    let vendored = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir set"))
        .join("vendor")
        .join(spec.file_name);
    vendored.is_file().then_some(vendored)
}
