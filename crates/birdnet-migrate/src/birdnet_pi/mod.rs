//! BirdNET-Pi source support.
//!
//! Provides `SchemaDetector`, `Migrator`, and `Validator` implementations
//! for the BirdNET-Pi `BirdDB.txt` SQLite format.

pub mod detector;
pub mod importer;
pub mod validator;

pub use detector::BirdNetPiDetector;
pub use importer::BirdNetPiImporter;
pub use validator::BirdNetPiValidator;

use std::path::Path;

use crate::error::MigrateError;
use crate::progress::ProgressHandle;
use crate::schema::DetectedSchema;
use crate::traits::{MigrationSummary, ValidationReport};

/// High-level entry point: detect, validate, and import a BirdNET-Pi database.
///
/// This is the function called by the web admin migration endpoint.
/// All three steps are run in sequence; validation failures are non-fatal
/// (warnings) unless `strict` is `true`.
///
/// The source file is **never modified**.
///
/// # Errors
///
/// Returns `MigrateError` if detection, validation (in strict mode), or
/// import fails.
pub fn run_migration(
    source_path: &Path,
    dest_path: &Path,
    strict: bool,
    progress: &ProgressHandle,
) -> Result<MigrationSummary, MigrateError> {
    use crate::traits::{Migrator, SchemaDetector, Validator};

    let detector = BirdNetPiDetector;
    let validator = BirdNetPiValidator;
    let importer = BirdNetPiImporter;

    // Step 1: Detect schema.
    let schema = detector.detect(source_path)?;

    if matches!(schema, DetectedSchema::BirdNetBehavior { .. }) && strict {
        return Err(MigrateError::UnsupportedSchema(
            "source is already a BirdNet-Behavior database — no migration needed".to_string(),
        ));
    }

    // Step 2: Validate source.
    let report = validator.validate_source(source_path)?;
    if !report.passed && strict {
        let failures: Vec<&str> = report
            .checks
            .iter()
            .filter(|c| !c.passed && c.required)
            .map(|c| c.detail.as_str())
            .collect();
        return Err(MigrateError::ValidationFailed(failures.join("; ")));
    }

    // Step 3: Import.
    importer.migrate(source_path, dest_path, progress)
}

/// Run validation only (without importing).  Used by the web UI pre-flight check.
///
/// # Errors
///
/// Returns `MigrateError` if the source cannot be opened.
pub fn validate_source(source_path: &Path) -> Result<(DetectedSchema, ValidationReport), MigrateError> {
    use crate::traits::{SchemaDetector, Validator};

    let detector = BirdNetPiDetector;
    let validator = BirdNetPiValidator;

    let schema = detector.detect(source_path)?;
    let report = validator.validate_source(source_path)?;
    Ok((schema, report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use tempfile::NamedTempFile;
    use crate::progress::ProgressHandle;

    fn make_source(n: usize) -> NamedTempFile {
        let tmp = NamedTempFile::new().unwrap();
        let conn = Connection::open(tmp.path()).unwrap();
        conn.execute_batch("CREATE TABLE detections (
            Date TEXT, Time TEXT, Sci_Name TEXT, Com_Name TEXT,
            Confidence REAL, Lat REAL, Lon REAL, Cutoff REAL,
            Week INTEGER, Sens REAL, Overlap REAL, File_Name TEXT);")
        .unwrap();
        for i in 0..n {
            conn.execute(
                "INSERT INTO detections VALUES (?1,'06:00:00','Turdus merula','Blackbird',0.9,NULL,NULL,NULL,NULL,NULL,NULL,NULL)",
                rusqlite::params![format!("2026-01-{:02}", (i % 28) + 1)],
            ).unwrap();
        }
        drop(conn);
        tmp
    }

    #[test]
    fn run_migration_end_to_end() {
        let src = make_source(20);
        let dst = NamedTempFile::new().unwrap();
        let progress = ProgressHandle::new();

        let summary = run_migration(src.path(), dst.path(), false, &progress).unwrap();
        assert_eq!(summary.source_rows, 20);
        assert_eq!(summary.imported_rows, 20);
    }

    #[test]
    fn validate_source_returns_schema_and_report() {
        let src = make_source(5);
        let (schema, report) = validate_source(src.path()).unwrap();
        assert!(matches!(schema, DetectedSchema::BirdNetPi { .. }));
        assert!(report.passed);
    }
}
