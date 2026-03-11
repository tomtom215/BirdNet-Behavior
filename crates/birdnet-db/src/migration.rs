//! Database schema migration framework.
//!
//! Uses a `schema_version` table to track applied migrations.
//! Migrations are defined as SQL strings and applied in order.

use rusqlite::Connection;
use std::fmt;

/// Migration errors.
#[derive(Debug)]
pub enum MigrationError {
    /// `SQLite` error during migration.
    Sqlite(rusqlite::Error),
    /// Migration logic error.
    Logic(String),
}

impl fmt::Display for MigrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(e) => write!(f, "migration sqlite error: {e}"),
            Self::Logic(msg) => write!(f, "migration error: {msg}"),
        }
    }
}

impl std::error::Error for MigrationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(e) => Some(e),
            Self::Logic(_) => None,
        }
    }
}

impl From<rusqlite::Error> for MigrationError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sqlite(e)
    }
}

/// A single database migration.
#[derive(Debug, Clone)]
pub struct Migration {
    /// Migration version number (must be sequential starting from 1).
    pub version: u32,
    /// Human-readable description.
    pub description: &'static str,
    /// SQL to apply the migration.
    pub up_sql: &'static str,
}

/// All known migrations, in order.
///
/// Add new migrations to the end of this list. Never modify existing migrations.
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        description: "Create detections table",
        up_sql: "CREATE TABLE IF NOT EXISTS detections (
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
    },
    Migration {
        version: 2,
        description: "Add indexes for common queries",
        up_sql: "CREATE INDEX IF NOT EXISTS idx_detections_date ON detections(Date);
                 CREATE INDEX IF NOT EXISTS idx_detections_species ON detections(Com_Name);
                 CREATE INDEX IF NOT EXISTS idx_detections_sci_name ON detections(Sci_Name);
                 CREATE INDEX IF NOT EXISTS idx_detections_confidence ON detections(Confidence);",
    },
    Migration {
        version: 3,
        description: "Add date-time composite index for time-range queries",
        up_sql: "CREATE INDEX IF NOT EXISTS idx_detections_datetime ON detections(Date, Time);",
    },
];

/// Ensure the `schema_version` tracking table exists.
fn ensure_version_table(conn: &Connection) -> Result<(), MigrationError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (
            version INTEGER PRIMARY KEY,
            description TEXT NOT NULL,
            applied_at TEXT NOT NULL DEFAULT (datetime('now'))
        );",
    )?;
    Ok(())
}

/// Get the current schema version (0 if no migrations applied).
///
/// # Errors
///
/// Returns `MigrationError` on query failure.
pub fn current_version(conn: &Connection) -> Result<u32, MigrationError> {
    ensure_version_table(conn)?;
    let version: u32 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_version",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    Ok(version)
}

/// Apply all pending migrations.
///
/// Returns the number of migrations applied.
///
/// # Errors
///
/// Returns `MigrationError` if any migration fails. Applied migrations
/// are committed individually, so partial progress is preserved.
pub fn migrate(conn: &Connection) -> Result<u32, MigrationError> {
    ensure_version_table(conn)?;
    let current = current_version(conn)?;
    let mut applied = 0;

    for migration in MIGRATIONS {
        if migration.version <= current {
            continue;
        }

        // Verify sequential ordering
        if migration.version != current + applied + 1 {
            return Err(MigrationError::Logic(format!(
                "expected migration version {}, found {}",
                current + applied + 1,
                migration.version
            )));
        }

        tracing::info!(
            version = migration.version,
            description = migration.description,
            "applying migration"
        );

        conn.execute_batch(migration.up_sql)?;
        conn.execute(
            "INSERT INTO schema_version (version, description) VALUES (?1, ?2)",
            rusqlite::params![migration.version, migration.description],
        )?;

        applied += 1;
    }

    if applied > 0 {
        tracing::info!(
            applied,
            new_version = current + applied,
            "migrations complete"
        );
    }

    Ok(applied)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .unwrap();
        conn
    }

    #[test]
    fn fresh_db_starts_at_version_zero() {
        let conn = memory_db();
        assert_eq!(current_version(&conn).unwrap(), 0);
    }

    #[test]
    fn migrate_applies_all_migrations() {
        let conn = memory_db();
        let applied = migrate(&conn).unwrap();
        assert_eq!(applied, MIGRATIONS.len() as u32);
        assert_eq!(current_version(&conn).unwrap(), MIGRATIONS.len() as u32);
    }

    #[test]
    fn migrate_is_idempotent() {
        let conn = memory_db();
        let first = migrate(&conn).unwrap();
        let second = migrate(&conn).unwrap();
        assert!(first > 0);
        assert_eq!(second, 0);
    }

    #[test]
    fn detections_table_exists_after_migration() {
        let conn = memory_db();
        migrate(&conn).unwrap();

        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
             VALUES ('2026-03-11', '08:30:00', 'Turdus merula', 'Eurasian Blackbird', 0.87)",
            [],
        )
        .unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM detections", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn indexes_exist_after_migration() {
        let conn = memory_db();
        migrate(&conn).unwrap();

        let index_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND tbl_name='detections'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            index_count >= 5,
            "expected at least 5 indexes, got {index_count}"
        );
    }

    #[test]
    fn version_table_tracks_history() {
        let conn = memory_db();
        migrate(&conn).unwrap();

        let rows: Vec<(u32, String)> = conn
            .prepare("SELECT version, description FROM schema_version ORDER BY version")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .filter_map(Result::ok)
            .collect();

        assert_eq!(rows.len(), MIGRATIONS.len());
        assert_eq!(rows[0].0, 1);
        assert_eq!(rows[0].1, "Create detections table");
    }
}
