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

// Re-export sub-module items at this level for backwards compatibility.
pub use analytics::*;
pub use sync::*;

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
    /// Non-fatal — the database can serve basic queries without the extension.
    ///
    /// # Errors
    ///
    /// Returns an error if the extension cannot be installed or loaded.
    pub fn load_extension(&mut self) -> Result<(), AnalyticsError> {
        self.conn
            .execute_batch(queries::LOAD_BEHAVIORAL)
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
}
