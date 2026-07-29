//! `DuckDB` file-based connection management.
//!
//! Provides a durable, file-backed `DuckDB` database for behavioral analytics.
//! Data is synced from the operational `SQLite` database (OLTP) into `DuckDB`
//! (OLAP) for complex analytical queries using the `duckdb-behavioral` extension.
//!
//! # Module layout
//!
//! | Sub-module   | Contents                                                     |
//! |--------------|--------------------------------------------------------------|
//! | `sync`       | `sync_from_sqlite`, `insert_detection`, count helpers       |
//! | `analytics`  | `sessionize`, `retention`, `funnel`, `next_species`         |

mod analytics;
#[cfg(test)]
mod live;
mod sync;

use duckdb::{Connection, Error as DuckDbError};
use std::fmt;
use std::path::{Path, PathBuf};

use crate::queries;

// Optional compile-time embed of the `behavioral` extension binary; the
// generated file declares `EMBEDDED_EXTENSION: Option<&[u8]>`. `Some(bytes)`
// when the release pipeline (or a maintainer's `vendor/`) supplies the
// extension, `None` otherwise — see `build.rs`.
include!(concat!(env!("OUT_DIR"), "/embedded_extension.rs"));

/// Errors from `DuckDB` operations.
#[derive(Debug)]
pub enum AnalyticsError {
    /// `DuckDB` connection or query error.
    Database(DuckDbError),
    /// Failed to load the behavioral extension.
    ExtensionLoad(String),
    /// Query returned unexpected data.
    InvalidData(String),
}

impl fmt::Display for AnalyticsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(e) => write!(f, "DuckDB error: {e}"),
            Self::ExtensionLoad(msg) => write!(f, "extension load error: {msg}"),
            Self::InvalidData(msg) => write!(f, "invalid data: {msg}"),
        }
    }
}

impl std::error::Error for AnalyticsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Database(e) => Some(e),
            Self::ExtensionLoad(_) | Self::InvalidData(_) => None,
        }
    }
}

impl From<DuckDbError> for AnalyticsError {
    fn from(e: DuckDbError) -> Self {
        Self::Database(e)
    }
}

/// Default cap on `DuckDB`'s buffer-pool memory.
///
/// Without an explicit limit `DuckDB` sizes its buffer pool at ~80% of system
/// RAM, which OOM-kills the whole process during a heavy analytics query on a
/// 1–4 GB Raspberry Pi. This conservative default leaves room for inference,
/// the web server, and the OS within the unit's memory budget; larger hosts
/// can raise it via the `BIRDNET_DUCKDB_MEMORY_LIMIT` environment variable.
const DEFAULT_DUCKDB_MEMORY_LIMIT: &str = "256MB";

/// Resolve the `DuckDB` memory limit from an optional configured value,
/// falling back to [`DEFAULT_DUCKDB_MEMORY_LIMIT`] when unset or malformed.
///
/// The value is interpolated into a `SET memory_limit='…'` statement, so only
/// well-formed literals (a leading digit then digits / unit letters / `.` /
/// `%`) are accepted — anything else falls back to the default. That both
/// keeps untrusted text out of the statement and avoids a startup failure from
/// an operator typo.
fn resolve_memory_limit(configured: Option<&str>) -> String {
    match configured.map(str::trim) {
        Some(v) if is_valid_memory_limit(v) => v.to_owned(),
        _ => DEFAULT_DUCKDB_MEMORY_LIMIT.to_owned(),
    }
}

/// Whether `v` is a safe `DuckDB` memory-limit literal: a leading ASCII digit
/// followed only by alphanumerics, `.`, or `%` (e.g. `512MB`, `2GB`, `80%`,
/// `1073741824`).
fn is_valid_memory_limit(v: &str) -> bool {
    v.bytes().next().is_some_and(|b| b.is_ascii_digit())
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'%')
}

/// A file-backed `DuckDB` connection for behavioral analytics.
#[derive(Debug)]
pub struct AnalyticsDb {
    pub(super) conn: Connection,
    path: PathBuf,
    extension_loaded: bool,
}

impl AnalyticsDb {
    /// Open or create a file-based `DuckDB` database.
    ///
    /// # Errors
    ///
    /// Returns an error if the database cannot be opened or the schema
    /// cannot be created.
    pub fn open(path: &Path) -> Result<Self, AnalyticsError> {
        // Open with `allow_unsigned_extensions=true` so the embedded-extension
        // fallback in `load_extension()` can `LOAD '<path>'` from a temp file.
        // DuckDB rejects changes to this setting once the connection is open,
        // so it must be a connection config at open time.
        let config = duckdb::Config::default()
            .allow_unsigned_extensions()
            .map_err(AnalyticsError::Database)?;
        let conn = Connection::open_with_flags(path, config)?;

        // Bound DuckDB's buffer memory before any query runs, so a heavy
        // analytics query cannot OOM the process on a small Pi (DuckDB
        // otherwise targets ~80% of system RAM).
        let memory_limit =
            resolve_memory_limit(std::env::var("BIRDNET_DUCKDB_MEMORY_LIMIT").ok().as_deref());
        conn.execute_batch(&format!("SET memory_limit='{memory_limit}';"))?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS detections (
                Date TEXT NOT NULL,
                Time TEXT NOT NULL,
                Sci_Name TEXT NOT NULL,
                Com_Name TEXT NOT NULL,
                Confidence DOUBLE NOT NULL,
                Lat DOUBLE,
                Lon DOUBLE,
                Cutoff DOUBLE,
                Week INTEGER,
                Sens DOUBLE,
                Overlap DOUBLE,
                File_Name TEXT
            );",
        )?;
        conn.execute_batch(queries::CREATE_DETECTIONS_TS_VIEW)?;
        Ok(Self {
            conn,
            path: path.to_path_buf(),
            extension_loaded: false,
        })
    }

    /// Get the database file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether the behavioral extension has been loaded.
    pub const fn extension_loaded(&self) -> bool {
        self.extension_loaded
    }

    /// Load the `duckdb-behavioral` extension.
    ///
    /// Three-stage fallback so the station works fully out of the box:
    ///   1. `LOAD behavioral` from DuckDB's local extension cache — offline,
    ///      succeeds when a prior install (or a previous run) already cached
    ///      the binary.
    ///   2. `INSTALL behavioral FROM community; LOAD behavioral` — fetches
    ///      from the community registry on first run when network is up.
    ///   3. A build-time-embedded copy of the extension binary, staged into a
    ///      temp file and loaded by path — the offline guarantee on a fresh
    ///      install whose Pi cannot reach the registry. Only available when
    ///      `build.rs` was given a binary (via `BIRDNET_BUNDLED_EXTENSION_FILE`
    ///      or `crates/birdnet-behavioral/vendor/behavioral.duckdb_extension`).
    ///
    /// Non-fatal at the caller: basic DuckDB queries still work without the
    /// extension; only the behavioural-specific functions (`sessionize`,
    /// `retention`, `window_funnel`, `sequence_*`) require it.
    ///
    /// # Errors
    ///
    /// Returns an error only when all three stages fail.
    pub fn load_extension(&mut self) -> Result<(), AnalyticsError> {
        // 1) Try the locally-cached LOAD (offline-safe).
        if self
            .conn
            .execute_batch(queries::LOAD_BEHAVIORAL_CACHED)
            .is_ok()
        {
            self.extension_loaded = true;
            return Ok(());
        }
        // 2) Try INSTALL FROM community (needs network on first run).
        if self.conn.execute_batch(queries::INSTALL_BEHAVIORAL).is_ok() {
            self.extension_loaded = true;
            return Ok(());
        }
        // 3) Final fallback: embedded binary from build.rs.
        if let Some(bytes) = EMBEDDED_EXTENSION {
            return self.load_embedded(bytes);
        }
        Err(AnalyticsError::ExtensionLoad(
            "behavioral extension not loaded: LOAD from cache failed, \
             INSTALL FROM community failed, and no embedded extension was bundled"
                .to_string(),
        ))
    }

    /// Stage embedded extension bytes to a temp file and `LOAD '<path>'`.
    ///
    /// The bytes themselves are the upstream community-signed build, but
    /// `LOAD` from an ad-hoc path bypasses DuckDB's signature check by design;
    /// `allow_unsigned_extensions=true` is set at open time (see `open`)
    /// because DuckDB refuses to change it on an already-open connection.
    fn load_embedded(&mut self, bytes: &[u8]) -> Result<(), AnalyticsError> {
        let dir = std::env::temp_dir().join("birdnet-behavioral-ext");
        std::fs::create_dir_all(&dir).map_err(|e| {
            AnalyticsError::ExtensionLoad(format!("create temp dir for embedded extension: {e}"))
        })?;
        let path = dir.join("behavioral.duckdb_extension");
        std::fs::write(&path, bytes)
            .map_err(|e| AnalyticsError::ExtensionLoad(format!("write embedded extension: {e}")))?;
        let escaped = path.display().to_string().replace('\'', "''");
        let sql = format!("LOAD '{escaped}';");
        self.conn
            .execute_batch(&sql)
            .map_err(|e| AnalyticsError::ExtensionLoad(format!("load embedded: {e}")))?;
        self.extension_loaded = true;
        tracing::info!(
            path = %path.display(),
            bytes = bytes.len(),
            "loaded behavioral extension from embedded bundle"
        );
        Ok(())
    }

    /// The bundled `DuckDB` engine version (e.g. `v1.5.5`).
    ///
    /// The `behavioral` community extension is version-locked to this exact
    /// `DuckDB` version — a build for any other version will not `LOAD` — so
    /// this is the value the published extension must target. Returns `None`
    /// only if the version string cannot be read.
    pub fn duckdb_version(&self) -> Option<String> {
        self.conn
            .query_row("SELECT version()", [], |r| r.get::<_, String>(0))
            .ok()
    }

    /// The loaded `behavioral` extension version (e.g. `v0.9.1`).
    ///
    /// Returns `None` when the extension is not loaded in this connection or its
    /// version is unavailable. Best-effort — any query error maps to `None` — so
    /// it is safe to call for status reporting regardless of extension state.
    pub fn extension_version(&self) -> Option<String> {
        self.conn
            .query_row(queries::BEHAVIORAL_EXTENSION_VERSION, [], |r| {
                r.get::<_, Option<String>>(0)
            })
            .ok()
            .flatten()
    }

    /// Force-reinstall the `behavioral` extension from the community registry
    /// and load it.
    ///
    /// Always re-downloads the latest build for the bundled `DuckDB` version,
    /// even when a cached copy exists. Backs the `--refresh-extension`
    /// maintenance command. Requires network access.
    ///
    /// # Errors
    ///
    /// Returns an error if the extension cannot be reinstalled or loaded — for
    /// example, offline, or the community registry has no build matching the
    /// bundled `DuckDB` version yet.
    pub fn refresh_extension(&mut self) -> Result<(), AnalyticsError> {
        self.conn
            .execute_batch(queries::FORCE_INSTALL_BEHAVIORAL)
            .map_err(|e| AnalyticsError::ExtensionLoad(e.to_string()))?;
        self.extension_loaded = true;
        Ok(())
    }

    /// Get a reference to the underlying `DuckDB` connection.
    pub const fn conn(&self) -> &Connection {
        &self.conn
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_db() -> (AnalyticsDb, TempDir) {
        let dir = TempDir::new().unwrap();
        let db = AnalyticsDb::open(&dir.path().join("analytics.duckdb")).unwrap();
        (db, dir)
    }

    #[test]
    fn embedded_extension_loads_when_bundled() {
        // Only meaningful when the build embedded a binary (via
        // `BIRDNET_BUNDLED_EXTENSION_FILE` or `vendor/`). Skips quietly
        // otherwise so the test is safe to run on un-bundled dev builds.
        let Some(bytes) = EMBEDDED_EXTENSION else {
            eprintln!("skipped — build did not embed an extension binary");
            return;
        };
        let (mut db, _tmp) = make_db();
        // Call the embedded path directly so the test exercises stage + LOAD
        // without depending on DuckDB's extension cache or network reachability.
        db.load_embedded(bytes)
            .expect("embedded extension should load via LOAD '<path>'");
        assert!(db.extension_loaded());
        assert!(
            db.extension_version().is_some(),
            "extension_version should report a version after the embedded load"
        );
    }

    #[test]
    fn open_creates_file() {
        let (db, _tmp) = make_db();
        assert!(db.path().exists());
        assert!(!db.extension_loaded());
    }

    #[test]
    fn duckdb_version_reports_bundled_engine() {
        let (db, _tmp) = make_db();
        let v = db.duckdb_version().expect("version() should be readable");
        // The bundled DuckDB is pinned to 1.5.x so it matches the DuckDB the
        // published `behavioral` community extension targets (see the duckdb
        // pin in the workspace Cargo.toml). If this trips, the pin moved and
        // the extension compatibility must be re-checked before shipping.
        assert!(v.starts_with("v1.5"), "unexpected DuckDB version: {v:?}");
    }

    #[test]
    fn extension_version_is_none_before_load() {
        // Not loaded in this connection, so there is no active version to
        // report — even if a build is cached in DuckDB's shared extension dir.
        let (db, _tmp) = make_db();
        assert!(!db.extension_loaded());
        assert_eq!(db.extension_version(), None);
    }

    #[test]
    fn memory_limit_defaults_when_unset_or_malformed() {
        assert_eq!(resolve_memory_limit(None), DEFAULT_DUCKDB_MEMORY_LIMIT);
        assert_eq!(resolve_memory_limit(Some("")), DEFAULT_DUCKDB_MEMORY_LIMIT);
        assert_eq!(
            resolve_memory_limit(Some("MB")),
            DEFAULT_DUCKDB_MEMORY_LIMIT
        ); // no leading digit
        // Anything that could break out of the SET statement falls back.
        assert_eq!(
            resolve_memory_limit(Some("256MB';DROP TABLE detections;--")),
            DEFAULT_DUCKDB_MEMORY_LIMIT
        );
    }

    #[test]
    fn memory_limit_accepts_well_formed_values() {
        assert_eq!(resolve_memory_limit(Some("512MB")), "512MB");
        assert_eq!(resolve_memory_limit(Some("2GB")), "2GB");
        assert_eq!(resolve_memory_limit(Some(" 80% ")), "80%");
        assert_eq!(resolve_memory_limit(Some("1073741824")), "1073741824");
    }

    #[test]
    fn open_applies_a_bounded_memory_limit() {
        let (db, _tmp) = make_db();
        let limit: String = db
            .conn()
            .query_row("SELECT current_setting('memory_limit')", [], |r| r.get(0))
            .unwrap();
        // Our 256MB cap reads back sub-GiB (e.g. "244.1 MiB"); crucially it is
        // NOT the multi-GiB ~80%-of-RAM default, so a big analytics query can't
        // OOM a small Pi.
        assert!(
            !limit.is_empty() && !limit.contains("GiB"),
            "expected a bounded sub-GiB memory_limit, got {limit:?}"
        );
    }
}
