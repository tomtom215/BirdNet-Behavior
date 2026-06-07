//! Source schema catalogue.
//!
//! Records all known source schemas (BirdNET-Pi versions, etc.) and provides
//! helpers for schema fingerprinting via table/column inspection.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;

use crate::error::MigrateError;

/// A detected source schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum DetectedSchema {
    /// BirdNET-Pi `BirdDB.txt` / `SQLite` (all known versions share the same schema).
    BirdNetPi {
        /// Row count in the `detections` table.
        row_count: u64,
    },
    /// BirdNET-Pi CSV/TSV detection log export.
    BirdNetPiCsv {
        /// Estimated row count (lines minus header).
        row_count: u64,
    },
    /// Identical schema to BirdNet-Behavior (already migrated or same format).
    BirdNetBehavior { row_count: u64 },
}

impl DetectedSchema {
    /// Human-readable name for this schema.
    pub const fn name(&self) -> &'static str {
        match self {
            Self::BirdNetPi { .. } => "BirdNET-Pi (BirdDB.txt / SQLite)",
            Self::BirdNetPiCsv { .. } => "BirdNET-Pi CSV/TSV detection log",
            Self::BirdNetBehavior { .. } => "BirdNet-Behavior",
        }
    }

    /// Row count in the detections table (or estimated line count for CSV).
    pub const fn row_count(&self) -> u64 {
        match self {
            Self::BirdNetPi { row_count }
            | Self::BirdNetPiCsv { row_count }
            | Self::BirdNetBehavior { row_count } => *row_count,
        }
    }

    /// Whether this schema requires CSV import.
    pub const fn is_csv(&self) -> bool {
        matches!(self, Self::BirdNetPiCsv { .. })
    }
}

/// Known column sets for fingerprinting (lowercase).
const BIRDNET_PI_COLUMNS: &[&str] = &[
    "date",
    "time",
    "sci_name",
    "com_name",
    "confidence",
    "lat",
    "lon",
    "cutoff",
    "week",
    "sens",
    "overlap",
    "file_name",
];

/// Open a `SQLite` file read-only and return the connection.
///
/// # Errors
///
/// Returns `MigrateError::SourceNotFound` if the file does not exist.
/// Returns `MigrateError::SourceOpen` if the file cannot be opened.
pub fn open_source_readonly(path: &Path) -> Result<Connection, MigrateError> {
    if !path.exists() {
        return Err(MigrateError::SourceNotFound(path.display().to_string()));
    }

    // Open the raw path with the READ_ONLY flag rather than building a
    // `file:…?mode=ro` URI. The path is operator-supplied (the admin migration
    // form), and with `SQLITE_OPEN_URI` set, a path containing `?` is parsed as
    // URI parameters that can override `mode=ro` (or select a writable VFS),
    // defeating the "source is never modified" guarantee. Without the URI flag
    // the `?` is just part of the filename, so READ_ONLY is authoritative.
    Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(MigrateError::SourceOpen)
}

/// List the tables present in the database.
///
/// # Errors
///
/// Returns `MigrateError::SourceOpen` on query failure.
pub fn list_tables(conn: &Connection) -> Result<Vec<String>, MigrateError> {
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .map_err(MigrateError::SourceOpen)?;

    let names = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(MigrateError::SourceOpen)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(MigrateError::SourceOpen)?;

    Ok(names)
}

/// Return the column names (lowercase) for the given table.
///
/// # Errors
///
/// Returns `MigrateError::SourceOpen` on query failure, or `MigrateError::CsvParse`
/// when `table` is not a valid SQL identifier.
pub fn column_names(conn: &Connection, table: &str) -> Result<Vec<String>, MigrateError> {
    // SQLite's `PRAGMA` accepts only identifier arguments — no bind parameters
    // — so `table` is unavoidably interpolated. Callers today pass the literal
    // "detections", but this function is `pub`, and `list_tables` returns names
    // straight from `sqlite_master` (attacker-influenced if the operator points
    // the migration at a hostile DB). Validate against a strict identifier
    // grammar so a malicious table name can't smuggle in arbitrary SQL.
    if !is_valid_identifier(table) {
        return Err(MigrateError::CsvParse(format!(
            "invalid table identifier: {table:?}"
        )));
    }
    let sql = format!("PRAGMA table_info({table})");
    let mut stmt = conn.prepare(&sql).map_err(MigrateError::SourceOpen)?;

    let cols = stmt
        .query_map([], |row| {
            let name: String = row.get(1)?;
            Ok(name.to_lowercase())
        })
        .map_err(MigrateError::SourceOpen)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(MigrateError::SourceOpen)?;

    Ok(cols)
}

/// Check whether `actual_cols` is a superset of `required_cols`.
pub fn has_required_columns(actual_cols: &[String], required_cols: &[&str]) -> bool {
    let actual: HashSet<&str> = actual_cols.iter().map(String::as_str).collect();
    required_cols.iter().all(|c| actual.contains(c))
}

/// Count rows in a table.
///
/// # Errors
///
/// Returns `MigrateError::SourceOpen` on query failure, or `MigrateError::CsvParse`
/// when `table` is not a valid SQL identifier.
pub fn row_count(conn: &Connection, table: &str) -> Result<u64, MigrateError> {
    if !is_valid_identifier(table) {
        return Err(MigrateError::CsvParse(format!(
            "invalid table identifier: {table:?}"
        )));
    }
    let sql = format!("SELECT COUNT(*) FROM {table}");
    let count: i64 = conn
        .query_row(&sql, [], |row| row.get(0))
        .map_err(MigrateError::SourceOpen)?;
    Ok(u64::try_from(count.max(0)).unwrap_or(0))
}

/// Return `true` if `s` is a valid SQL identifier (first char `[A-Za-z_]`,
/// rest `[A-Za-z0-9_]`). Used by helpers that must interpolate a table name
/// into a query (`PRAGMA table_info`, `SELECT FROM`) — bind params don't work
/// for identifiers — to keep them safe in the face of attacker-influenced
/// table names (e.g. names returned by `list_tables` against a hostile DB).
fn is_valid_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Detect the schema of the database at `path`.
///
/// Returns `DetectedSchema` for known schemas, or `MigrateError::UnknownSchema`.
///
/// # Errors
///
/// Returns `MigrateError::SourceNotFound` if the file does not exist.
/// Returns `MigrateError::SourceOpen` if the database cannot be opened.
/// Returns `MigrateError::UnknownSchema` if no known schema matches.
pub fn detect_schema(path: &Path) -> Result<DetectedSchema, MigrateError> {
    let conn = open_source_readonly(path)?;
    let tables = list_tables(&conn)?;

    if !tables.iter().any(|t| t.eq_ignore_ascii_case("detections")) {
        return Err(MigrateError::UnknownSchema(format!(
            "no 'detections' table found; tables present: {}",
            tables.join(", ")
        )));
    }

    let cols = column_names(&conn, "detections")?;
    let count = row_count(&conn, "detections")?;

    if has_required_columns(&cols, BIRDNET_PI_COLUMNS) {
        // BirdNet-Behavior uses the same column set — distinguish by checking
        // for a 'schema_version' table (only present in BirdNet-Behavior dbs).
        if tables
            .iter()
            .any(|t| t.eq_ignore_ascii_case("schema_version"))
        {
            Ok(DetectedSchema::BirdNetBehavior { row_count: count })
        } else {
            Ok(DetectedSchema::BirdNetPi { row_count: count })
        }
    } else {
        Err(MigrateError::UnknownSchema(format!(
            "unrecognised column set: {}",
            cols.join(", ")
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn identifier_validation_accepts_normal_names() {
        assert!(is_valid_identifier("detections"));
        assert!(is_valid_identifier("Detections"));
        assert!(is_valid_identifier("_private"));
        assert!(is_valid_identifier("name_42"));
    }

    #[test]
    fn identifier_validation_rejects_injection_attempts() {
        assert!(!is_valid_identifier(""));
        assert!(!is_valid_identifier("1abc")); // starts with digit
        assert!(!is_valid_identifier("a; DROP TABLE x;"));
        assert!(!is_valid_identifier("a\"b"));
        assert!(!is_valid_identifier("a b"));
        assert!(!is_valid_identifier("'OR 1=1--"));
        assert!(!is_valid_identifier("a)"));
    }

    #[test]
    fn column_names_rejects_invalid_identifier() {
        let tmp = NamedTempFile::new().unwrap();
        let conn = Connection::open(tmp.path()).unwrap();
        let err = column_names(&conn, "a; DROP TABLE x").expect_err("must reject");
        assert!(
            matches!(err, MigrateError::CsvParse(_)),
            "expected CsvParse, got {err:?}"
        );
    }

    fn make_birdnet_pi_db() -> (NamedTempFile, Connection) {
        let tmp = NamedTempFile::new().unwrap();
        let conn = Connection::open(tmp.path()).unwrap();
        conn.execute_batch(
            "CREATE TABLE detections (
                Date TEXT, Time TEXT, Sci_Name TEXT, Com_Name TEXT,
                Confidence REAL, Lat REAL, Lon REAL, Cutoff REAL,
                Week INTEGER, Sens REAL, Overlap REAL, File_Name TEXT
            );
            INSERT INTO detections VALUES
                ('2026-01-01', '06:00:00', 'Turdus merula', 'Eurasian Blackbird',
                 0.9, 51.5, -0.1, 0.7, 1, 1.0, 0.0, 'rec.wav');",
        )
        .unwrap();
        (tmp, conn)
    }

    #[test]
    fn detects_birdnet_pi_schema() {
        let (tmp, _conn) = make_birdnet_pi_db();
        let schema = detect_schema(tmp.path()).unwrap();
        assert!(matches!(schema, DetectedSchema::BirdNetPi { row_count: 1 }));
    }

    #[test]
    fn unknown_schema_no_detections_table() {
        let tmp = NamedTempFile::new().unwrap();
        let conn = Connection::open(tmp.path()).unwrap();
        conn.execute_batch("CREATE TABLE foo (id INTEGER);")
            .unwrap();
        drop(conn);

        let err = detect_schema(tmp.path()).unwrap_err();
        assert!(matches!(err, MigrateError::UnknownSchema(_)));
    }

    #[test]
    fn source_not_found() {
        let err = detect_schema(Path::new("/nonexistent/birds.db")).unwrap_err();
        assert!(matches!(err, MigrateError::SourceNotFound(_)));
    }

    #[test]
    fn has_required_columns_pass() {
        let actual: Vec<String> = BIRDNET_PI_COLUMNS
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        assert!(has_required_columns(&actual, BIRDNET_PI_COLUMNS));
    }

    #[test]
    fn has_required_columns_missing() {
        let actual = vec!["date".to_string(), "time".to_string()];
        assert!(!has_required_columns(&actual, BIRDNET_PI_COLUMNS));
    }
}
