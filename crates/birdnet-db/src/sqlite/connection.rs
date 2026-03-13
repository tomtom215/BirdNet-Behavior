//! SQLite connection helpers and error types.
//!
//! Provides WAL-mode-enforced connection opening for both existing and new
//! databases, plus `quick_check` for integrity verification.

use rusqlite::Connection;
use std::fmt;
use std::path::Path;

/// Database errors.
#[derive(Debug)]
pub enum DbError {
    /// `SQLite` error.
    Sqlite(rusqlite::Error),
    /// Database file not found.
    NotFound(String),
    /// Schema validation failed.
    Schema(String),
}

impl fmt::Display for DbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(e) => write!(f, "sqlite error: {e}"),
            Self::NotFound(path) => write!(f, "database not found: {path}"),
            Self::Schema(msg) => write!(f, "schema error: {msg}"),
        }
    }
}

impl std::error::Error for DbError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(e) => Some(e),
            Self::NotFound(_) | Self::Schema(_) => None,
        }
    }
}

impl From<rusqlite::Error> for DbError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sqlite(e)
    }
}

/// Recommended PRAGMAs applied to every connection.
const PRAGMAS: &str = "PRAGMA journal_mode=WAL;
 PRAGMA synchronous=NORMAL;
 PRAGMA busy_timeout=5000;
 PRAGMA cache_size=-2000;
 PRAGMA foreign_keys=ON;";

/// Open a `SQLite` connection with WAL mode and recommended PRAGMAs.
///
/// The database file must already exist.
///
/// # Errors
///
/// Returns `DbError::NotFound` if the path does not exist.
/// Returns `DbError::Sqlite` if WAL mode cannot be set.
pub fn open_connection(path: &Path) -> Result<Connection, DbError> {
    if !path.exists() {
        return Err(DbError::NotFound(path.display().to_string()));
    }
    let conn = Connection::open(path)?;
    conn.execute_batch(PRAGMAS)?;
    Ok(conn)
}

/// Open or create a `SQLite` database with the detections schema.
///
/// Creates the file and schema if it does not yet exist; opens it
/// read-write if it does.
///
/// # Errors
///
/// Returns `DbError` on connection or schema creation failure.
pub fn open_or_create(path: &Path) -> Result<Connection, DbError> {
    let conn = Connection::open(path)?;
    conn.execute_batch(PRAGMAS)?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS detections (
            Date TEXT NOT NULL,
            Time TEXT NOT NULL,
            Sci_Name TEXT NOT NULL,
            Com_Name TEXT NOT NULL,
            Confidence REAL NOT NULL,
            Lat REAL,
            Lon REAL,
            Cutoff REAL,
            Week INTEGER,
            Sens REAL,
            Overlap REAL,
            File_Name TEXT,
            UNIQUE(Date, Time, Sci_Name)
        );",
    )?;
    Ok(conn)
}

/// Run a quick integrity check.
///
/// # Errors
///
/// Returns `DbError` on check failure.
pub fn quick_check(conn: &Connection) -> Result<bool, DbError> {
    let result: String = conn.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    Ok(result == "ok")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_db() -> (tempfile::NamedTempFile, Connection) {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_or_create(tmp.path()).unwrap();
        (tmp, conn)
    }

    #[test]
    fn wal_mode_is_set() {
        let (_tmp, conn) = temp_db();
        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
    }

    #[test]
    fn quick_check_passes() {
        let (_tmp, conn) = temp_db();
        assert!(quick_check(&conn).unwrap());
    }

    #[test]
    fn open_nonexistent_returns_error() {
        let result = open_connection(&PathBuf::from("/nonexistent/birds.db"));
        assert!(matches!(result, Err(DbError::NotFound(_))));
    }

    #[test]
    fn open_or_create_twice_is_idempotent() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let _c1 = open_or_create(tmp.path()).unwrap();
        let _c2 = open_or_create(tmp.path()).unwrap();
    }
}
