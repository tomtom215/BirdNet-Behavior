//! `SQLite` connection helpers and error types.
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

impl From<crate::migration::MigrationError> for DbError {
    fn from(e: crate::migration::MigrationError) -> Self {
        match e {
            crate::migration::MigrationError::Sqlite(s) => Self::Sqlite(s),
            crate::migration::MigrationError::Logic(msg) => Self::Schema(msg),
        }
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
    // Apply the full migration chain so the resulting database matches the
    // production schema exactly. Previously this function hand-coded a
    // bare-bones `CREATE TABLE detections` with only the migration-1 columns
    // and the old UNIQUE constraint — every test fixture using this helper
    // then missed every later column (`is_locked`, `chunk_offset_secs`) and
    // the relaxed UNIQUE key from migration 11, so tests silently passed
    // against a schema that didn't exist in production.
    crate::migration::migrate(&conn)?;
    Ok(conn)
}

/// PRAGMAs for a read-only connection.
///
/// Deliberately not [`PRAGMAS`]. `journal_mode` is a property of the database
/// file, not of the connection, and setting it needs write access — a read-only
/// connection either errors or silently reports the existing mode, so asking is
/// noise at best. `synchronous` governs writes and has nothing to say here.
/// What is left matters: `busy_timeout` so a checkpoint does not turn a read
/// into an instant failure, `cache_size` matched to the writer's so a reader's
/// memory is not a surprise, and `foreign_keys` for parity in case a read path
/// ever consults them.
const READ_PRAGMAS: &str = "PRAGMA busy_timeout=5000;
 PRAGMA cache_size=-2000;
 PRAGMA foreign_keys=ON;";

/// Open an existing database **read-only**.
///
/// # What this is for
///
/// WAL lets any number of readers run concurrently with one writer. A process
/// holding a single connection gets none of that, because the connection itself
/// is the bottleneck — which is what `birdnet-web`'s reader pool exists to fix.
///
/// Read-only is not a formality here. It is the only thing that makes the split
/// safe to introduce incrementally: a write accidentally routed down the read
/// path fails immediately and loudly with `attempt to write a readonly
/// database`, in the first test that exercises it, rather than working by luck
/// until two of them interleave.
///
/// # Errors
///
/// Returns [`DbError::NotFound`] if the path does not exist — a read-only
/// connection cannot create the file, so a missing one is a caller error rather
/// than something to paper over. Returns [`DbError::Sqlite`] if the file cannot
/// be opened.
pub fn open_readonly(path: &Path) -> Result<Connection, DbError> {
    if !path.exists() {
        return Err(DbError::NotFound(path.display().to_string()));
    }
    let conn = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )?;
    conn.execute_batch(READ_PRAGMAS)?;
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
