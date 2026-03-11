//! SQLite operational database.
//!
//! Provides connection management, WAL mode enforcement, and query helpers
//! for the birds.db detection database.

use rusqlite::{params, Connection};
use std::fmt;
use std::path::Path;

/// Database errors.
#[derive(Debug)]
pub enum DbError {
    /// SQLite error.
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

/// Open a SQLite connection with WAL mode and recommended PRAGMAs.
///
/// # Errors
///
/// Returns `DbError` if the database cannot be opened or WAL mode cannot be set.
pub fn open_connection(path: &Path) -> Result<Connection, DbError> {
    if !path.exists() {
        return Err(DbError::NotFound(path.display().to_string()));
    }

    let conn = Connection::open(path)?;

    // Enforce WAL mode for crash resilience
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;
         PRAGMA busy_timeout=5000;
         PRAGMA cache_size=-2000;
         PRAGMA foreign_keys=ON;",
    )?;

    Ok(conn)
}

/// Open or create a SQLite database with the detections schema.
///
/// # Errors
///
/// Returns `DbError` on connection or schema creation failure.
pub fn open_or_create(path: &Path) -> Result<Connection, DbError> {
    let conn = Connection::open(path)?;

    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;
         PRAGMA busy_timeout=5000;
         PRAGMA cache_size=-2000;
         PRAGMA foreign_keys=ON;",
    )?;

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
            File_Name TEXT
        );",
    )?;

    Ok(conn)
}

/// A detection record for database insertion.
#[derive(Debug, Clone)]
pub struct DetectionRecord<'a> {
    /// Detection date (YYYY-MM-DD).
    pub date: &'a str,
    /// Detection time (HH:MM:SS).
    pub time: &'a str,
    /// Scientific name.
    pub sci_name: &'a str,
    /// Common name.
    pub com_name: &'a str,
    /// Confidence score.
    pub confidence: f64,
    /// Latitude.
    pub lat: &'a str,
    /// Longitude.
    pub lon: &'a str,
    /// Confidence cutoff threshold.
    pub cutoff: &'a str,
    /// ISO week number.
    pub week: &'a str,
    /// Sensitivity setting.
    pub sensitivity: &'a str,
    /// Overlap setting.
    pub overlap: &'a str,
    /// Extracted audio filename.
    pub file_name: &'a str,
}

/// Insert a detection record into the database.
///
/// # Errors
///
/// Returns `DbError` on insert failure.
pub fn insert_detection(conn: &Connection, record: &DetectionRecord<'_>) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO detections VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            record.date,
            record.time,
            record.sci_name,
            record.com_name,
            record.confidence,
            record.lat,
            record.lon,
            record.cutoff,
            record.week,
            record.sensitivity,
            record.overlap,
            record.file_name,
        ],
    )?;
    Ok(())
}

/// Get the total number of detections.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn detection_count(conn: &Connection) -> Result<i64, DbError> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM detections", [], |row| row.get(0))?;
    Ok(count)
}

/// Get the number of unique species.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn species_count(conn: &Connection) -> Result<i64, DbError> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(DISTINCT Sci_Name) FROM detections",
        [],
        |row| row.get(0),
    )?;
    Ok(count)
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
    fn create_and_insert() {
        let (_tmp, conn) = temp_db();
        let record = DetectionRecord {
            date: "2026-03-11",
            time: "08:30:00",
            sci_name: "Turdus merula",
            com_name: "Eurasian Blackbird",
            confidence: 0.87,
            lat: "42.36",
            lon: "-71.06",
            cutoff: "0.7",
            week: "10",
            sensitivity: "1.25",
            overlap: "0.0",
            file_name: "test.wav",
        };
        insert_detection(&conn, &record).unwrap();

        assert_eq!(detection_count(&conn).unwrap(), 1);
        assert_eq!(species_count(&conn).unwrap(), 1);
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
}
