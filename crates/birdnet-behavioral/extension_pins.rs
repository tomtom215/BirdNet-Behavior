//! Which DuckDB extension builds this workspace is willing to embed.
//!
//! The extensions are native code: `build.rs` bakes them into the binary with
//! `include_bytes!` and the runtime `LOAD`s them from a temp file into a
//! connection opened with `allow_unsigned_extensions` (an ad-hoc-path `LOAD`
//! bypasses DuckDB's own signature check; see `connection/mod.rs`). They used to
//! be fetched by an unchecked `curl` in four places — the release workflow, the
//! Dockerfile, CI and the upgrade test — and `build.rs` only checked that the
//! footer *parsed* and targeted the right engine, not that the bytes were the
//! ones intended. Whatever the CDN served that day was compiled into the
//! release and the image, with nothing recorded, and a failed download shipped a
//! build without ICU (the 0.13.1 fault: every dashboard empty for days).
//!
//! The models are pinned by sha256 and refused on mismatch; this is the same
//! discipline for the one other thing the project downloads and executes. The
//! build refuses to embed an extension whose digest is not in [`PINS`], and
//! refuses one whose digest differs from its pin.
//!
//! # Updating
//!
//! When the bundled DuckDB version moves (the `duckdb` pin in the root
//! `Cargo.toml`), or DuckDB republishes a build:
//!
//! 1. Download the new `.gz` from the registry named in `build.rs`, `gunzip` it.
//! 2. Check it is genuine: load it into a DuckDB of the *same version* with
//!    unsigned extensions disallowed (the default). The signature is verified
//!    against the keys built into that DuckDB, so this is the one step that is
//!    not just trusting the CDN. The `duckdb` crate this workspace links is such
//!    a DuckDB: `Connection::open_in_memory_with_flags(Config::default())`, then
//!    `LOAD '/path/to/<name>.duckdb_extension'` (the file must be called
//!    `<name>.duckdb_extension`, because DuckDB derives the entry point from the
//!    name). Only a build for the machine's own platform can be loaded; for the
//!    others the pin is trust-on-first-use, and says so below.
//! 3. Take `sha256sum` of the *decompressed* file, and the footer fields
//!    (`build.rs` prints them as `embedding <name> <version> (<abi> ABI, version
//!    target <target>, <platform>)`), and edit [`PINS`].
//!
//! # What was verified when these were pinned (2026-10-14, DuckDB v1.5.6)
//!
//! * `linux_amd64` `icu` and `behavioral`: loaded by the DuckDB 1.5.6 this
//!   workspace links, with unsigned extensions disallowed, which verifies the
//!   signature; a copy of each with one bit changed was refused. (Run from a
//!   throwaway test in this crate against freshly downloaded files whose digests
//!   equal the rows below.)
//! * `linux_arm64` and `osx_arm64`: **not** loadable here (wrong platform), so
//!   their digests are what the registry served that day — trust on first use.
//!   They are still pinned, because a later change is then a visible failure.

/// One extension build the workspace may embed.
pub struct Pin {
    /// DuckDB's name for it: `behavioral` or `icu`.
    pub name: &'static str,
    /// The extension's own version from its footer, e.g. `v0.10.0`.
    pub extension_version: &'static str,
    /// The version the engine checks: a DuckDB version for `CPP` extensions, a
    /// C API version for `C_STRUCT` ones (see `build.rs`).
    pub version_target: &'static str,
    /// Target platform, e.g. `linux_amd64`.
    pub platform: &'static str,
    /// Lower-case hex SHA-256 of the *decompressed* extension file.
    pub sha256: &'static str,
}

/// The builds this workspace will embed. Each is the file
/// `https://<registry>/v1.5.6/<platform>/<name>.duckdb_extension.gz`
/// decompressed; see the module header for how to add or replace one.
pub const PINS: &[Pin] = &[
    Pin {
        name: "behavioral",
        extension_version: "v0.10.0",
        version_target: "v1.2.0",
        platform: "linux_amd64",
        sha256: "da51b95086c8fe3ed3e3b2ce5c89f38d5eb5344e52538d87a8f934678d973591",
    },
    Pin {
        name: "behavioral",
        extension_version: "v0.10.0",
        version_target: "v1.2.0",
        platform: "linux_arm64",
        sha256: "c13812ee7c3a58d58b0b70a7f382891685d09c7822a9436e59ca7c27cf07109f",
    },
    Pin {
        name: "icu",
        extension_version: "v1.5.6",
        version_target: "v1.5.6",
        platform: "linux_amd64",
        sha256: "f7bdb7b30da48406cc58429b8b5d901a1aea5f8b9c0bd1928642f0ab262d5d85",
    },
    Pin {
        name: "icu",
        extension_version: "v1.5.6",
        version_target: "v1.5.6",
        platform: "linux_arm64",
        sha256: "414bd5f36d5efd15cf92d008a7e9369ada3031d584e8a992bff73f9d3b444749",
    },
    Pin {
        name: "icu",
        extension_version: "v1.5.6",
        version_target: "v1.5.6",
        platform: "osx_arm64",
        sha256: "3578a29bb3e05024eaf75c849d502089d25ed2b629823ed97508a86e62acbba7",
    },
];

/// What [`check`] found.
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The bytes are exactly the pinned build.
    Pinned,
    /// A build is pinned for this name, version and platform, and these are not
    /// its bytes.
    Mismatch {
        /// The digest the pin names.
        expected: &'static str,
    },
    /// No pin names this build at all.
    Unpinned,
}

/// Whether an extension whose footer says `(name, extension_version,
/// version_target, platform)` and whose decompressed bytes hash to `sha256` is
/// one of the builds in [`PINS`].
#[must_use]
pub fn check(
    name: &str,
    extension_version: &str,
    version_target: &str,
    platform: &str,
    sha256: &str,
) -> Verdict {
    PINS.iter()
        .find(|p| {
            p.name == name
                && p.extension_version == extension_version
                && p.version_target == version_target
                && p.platform == platform
        })
        .map_or(Verdict::Unpinned, |p| {
            if p.sha256 == sha256 {
                Verdict::Pinned
            } else {
                Verdict::Mismatch { expected: p.sha256 }
            }
        })
}

#[cfg(test)]
mod tests {
    use super::{PINS, Verdict, check};

    /// A build the table names is accepted.
    #[test]
    fn every_pin_accepts_its_own_bytes() {
        for p in PINS {
            assert_eq!(
                check(
                    p.name,
                    p.extension_version,
                    p.version_target,
                    p.platform,
                    p.sha256
                ),
                Verdict::Pinned,
                "{} {} {}",
                p.name,
                p.extension_version,
                p.platform
            );
        }
    }

    /// The counterpart, and the point of the table: the same build with other
    /// bytes is a mismatch that names the digest it wanted, not "unpinned" and
    /// not accepted.
    #[test]
    fn the_same_build_with_other_bytes_is_refused() {
        let other = "0".repeat(64);
        for p in PINS {
            assert_eq!(
                check(
                    p.name,
                    p.extension_version,
                    p.version_target,
                    p.platform,
                    &other
                ),
                Verdict::Mismatch { expected: p.sha256 },
                "{} {}",
                p.name,
                p.platform
            );
        }
    }

    /// A build no pin names is neither accepted nor called a mismatch: a new
    /// extension version, platform or name is a different decision from a
    /// changed file, and the build script words them differently.
    #[test]
    fn a_build_nothing_names_is_unpinned() {
        let p = &PINS[0];
        let any = p.sha256;
        assert_eq!(
            check("behavioral", "v9.9.9", p.version_target, p.platform, any),
            Verdict::Unpinned
        );
        assert_eq!(
            check("behavioral", p.extension_version, "v9.9.9", p.platform, any),
            Verdict::Unpinned
        );
        assert_eq!(
            check(
                "behavioral",
                p.extension_version,
                p.version_target,
                "windows_amd64",
                any
            ),
            Verdict::Unpinned
        );
        assert_eq!(
            check(
                "not_an_extension",
                p.extension_version,
                p.version_target,
                p.platform,
                any
            ),
            Verdict::Unpinned
        );
    }

    /// Hand-edited tables rot in small ways: a digest pasted with a trailing
    /// newline, in upper case, or from the compressed file would otherwise only
    /// show up as a mismatch on the first build that tried to use it.
    #[test]
    fn the_table_is_well_formed() {
        for p in PINS {
            assert_eq!(
                p.sha256.len(),
                64,
                "{} {}: not a sha256",
                p.name,
                p.platform
            );
            assert!(
                p.sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                "{} {}: digest must be lower-case hex",
                p.name,
                p.platform
            );
            assert!(
                p.extension_version.starts_with('v'),
                "{} {}",
                p.name,
                p.platform
            );
            assert!(
                p.version_target.starts_with('v'),
                "{} {}",
                p.name,
                p.platform
            );
        }
        let mut keys: Vec<_> = PINS
            .iter()
            .map(|p| (p.name, p.extension_version, p.version_target, p.platform))
            .collect();
        let before = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), before, "two pins name the same build");
        let mut digests: Vec<_> = PINS.iter().map(|p| p.sha256).collect();
        digests.sort_unstable();
        digests.dedup();
        assert_eq!(
            digests.len(),
            before,
            "two builds share a digest: a pasted row"
        );
    }
}
