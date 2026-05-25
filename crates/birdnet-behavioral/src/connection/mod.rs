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
mod sync;

use duckdb::{Connection, Error as DuckDbError};
use std::fmt;
use std::path::{Path, PathBuf};

use crate::queries;

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
        let conn = Connection::open(path)?;

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
    /// Tries loading from the local cache first (works offline), then falls
    /// back to installing from the DuckDB community registry (requires network
    /// on first run only). Non-fatal — the database can serve basic queries
    /// without the extension.
    ///
    /// # Errors
    ///
    /// Returns an error if the extension cannot be installed or loaded.
    pub fn load_extension(&mut self) -> Result<(), AnalyticsError> {
        // Try loading from local cache first (offline-safe).
        if self
            .conn
            .execute_batch(queries::LOAD_BEHAVIORAL_CACHED)
            .is_ok()
        {
            self.extension_loaded = true;
            return Ok(());
        }

        // Not cached — install from community registry (requires network).
        self.conn
            .execute_batch(queries::INSTALL_BEHAVIORAL)
            .map_err(|e| AnalyticsError::ExtensionLoad(e.to_string()))?;
        self.extension_loaded = true;
        Ok(())
    }

    /// The bundled `DuckDB` engine version (e.g. `v1.5.1`).
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

    /// The loaded `behavioral` extension version (e.g. `v0.4.0`).
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
