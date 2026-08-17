//! BirdNET-Pi source support.
//!
//! Provides `SchemaDetector`, `Migrator`, and `Validator` implementations
//! for the BirdNET-Pi `BirdDB.txt` `SQLite` format **and** the CSV/TSV
//! detection log export.
//!
//! | Format | Extension(s) | Importer |
//! |--------|-------------|----------|
//! | SQLite | `.db`, `.txt`, `.sqlite` | [`BirdNetPiImporter`] |
//! | CSV/TSV | `.csv`, `.tsv`, `.txt` (tab-delimited) | [`CsvImporter`] |

pub mod csv_importer;
pub mod detector;
pub mod importer;
pub mod species_report;
pub mod validator;

pub use csv_importer::CsvImporter;
pub use detector::BirdNetPiDetector;
pub use importer::BirdNetPiImporter;
pub use species_report::{
    MigrationReport, PostMigrationReport, SpeciesDiff, SpeciesStats, compare_source_dest,
    generate_report,
};
pub use validator::BirdNetPiValidator;

use std::path::Path;

use crate::error::MigrateError;
use crate::progress::ProgressHandle;
use crate::schema::DetectedSchema;
use crate::traits::{MigrationSummary, ValidationReport};

/// High-level entry point: detect, validate, and import a BirdNET-Pi database
/// **or** CSV/TSV detection log.
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
    run_migration_with_options(
        source_path,
        dest_path,
        strict,
        progress,
        &crate::provenance::ImportOptions::default(),
        (None, None),
    )
}

/// [`run_migration`] plus the reconciliation the operator chose.
///
/// `options` carries the clock shift and the label for the source station;
/// `station` is this station's own `(lat, lon)` so the recorded batch can say
/// how far apart the two sites are. Both are recorded whether or not they are
/// set, because "we did not know" is itself worth being able to read back.
///
/// # Errors
///
/// Returns `MigrateError` if detection, validation (in strict mode), or import
/// fails.
pub fn run_migration_with_options(
    source_path: &Path,
    dest_path: &Path,
    strict: bool,
    progress: &ProgressHandle,
    options: &crate::provenance::ImportOptions,
    station: (Option<f64>, Option<f64>),
) -> Result<MigrationSummary, MigrateError> {
    use crate::traits::{Migrator, SchemaDetector, Validator};

    // CSV/TSV path — if the file is not a SQLite database, try CSV import.
    if is_csv_file(source_path) {
        return CsvImporter.migrate(source_path, dest_path, progress);
    }

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

    // Step 3: Import, carrying the operator's reconciliation choices so the
    // merged history stays attributable to the site and clock it came from.
    importer.migrate_with_options(source_path, dest_path, progress, options, station)
}

/// Run validation only (without importing).  Used by the web UI pre-flight check.
///
/// Returns a tuple of `(schema, validation_report, migration_report)` for
/// the admin UI to display a comprehensive pre-migration preview.
///
/// For CSV files, returns an estimated schema and a minimal validation report.
///
/// # Errors
///
/// Returns `MigrateError` if the source cannot be opened.
pub fn validate_source(
    source_path: &Path,
) -> Result<(DetectedSchema, ValidationReport, MigrationReport), MigrateError> {
    validate_source_against_station(source_path, None, None)
}

/// [`validate_source`] with the station's own coordinates, so the report can
/// say whether the source came from *here*.
///
/// Split out rather than folded in because the migrate crate has no access to
/// the station's settings — those live in the destination database, which
/// validation deliberately does not open. The caller supplies them.
///
/// # Errors
///
/// Returns `MigrateError` if the source cannot be opened.
pub fn validate_source_against_station(
    source_path: &Path,
    station_lat: Option<f64>,
    station_lon: Option<f64>,
) -> Result<(DetectedSchema, ValidationReport, MigrationReport), MigrateError> {
    use crate::traits::{SchemaDetector, Validator};

    if is_csv_file(source_path) {
        let (schema, report) = validate_csv_source(source_path)?;
        let migration_report = MigrationReport {
            total_rows: i64::try_from(schema.row_count()).unwrap_or(i64::MAX),
            unique_species: 0,
            date_range: None,
            top_species: vec![],
            null_date_rows: 0,
            invalid_confidence_rows: 0,
            duplicate_rows: 0,
            quality_ok: report.passed,
        };
        return Ok((schema, report, migration_report));
    }

    let detector = BirdNetPiDetector;
    let validator = BirdNetPiValidator;

    let schema = detector.detect(source_path)?;
    let mut report = validator.validate_source(source_path)?;

    // The check the four original ones never made: is this the same place?
    // Appended here rather than inside the Validator so it stays a pure
    // source-vs-station comparison; the validator itself never sees the
    // destination.
    // `passed` is recomputed the way `ValidationReport::new` does — a check
    // that is not `required` never flips it. That matters here: the location
    // check is advisory by design, and a station that has simply not set its
    // coordinates yet must not find strict-mode imports blocked by a check that
    // could not be performed.
    let profile = crate::provenance::SourceProfile::read(source_path)?;
    report.checks.push(crate::provenance::location_check(
        &profile,
        station_lat,
        station_lon,
    ));
    report.passed = report.checks.iter().all(|c| c.passed || !c.required);

    let migration_report = generate_report(source_path)?;
    Ok((schema, report, migration_report))
}

/// Cheap CSV validation: check file exists, is readable, has ≥1 data line.
fn validate_csv_source(path: &Path) -> Result<(DetectedSchema, ValidationReport), MigrateError> {
    use crate::traits::{ValidationCheck, ValidationReport};
    use std::io::{BufRead, BufReader};

    let file = std::fs::File::open(path).map_err(MigrateError::Io)?;
    let reader = BufReader::new(file);
    let mut lines = reader.lines();

    let Some(Ok(header)) = lines.next() else {
        return Ok((
            DetectedSchema::BirdNetPiCsv { row_count: 0 },
            ValidationReport::new(
                "BirdNET-Pi CSV",
                0,
                vec![ValidationCheck::fail(
                    "readable",
                    "file is empty or unreadable",
                    true,
                )],
            ),
        ));
    };

    let delim = if header.contains('\t') { '\t' } else { ',' };
    let fields: Vec<&str> = header.splitn(6, delim).collect();
    let has_expected_header = fields.len() >= 5
        && fields[0].trim().eq_ignore_ascii_case("date")
        && fields[3].trim().eq_ignore_ascii_case("com_name");

    let row_count = lines
        .filter(|l| l.as_ref().is_ok_and(|s| !s.trim().is_empty()))
        .count() as u64;

    let checks = vec![
        if has_expected_header {
            ValidationCheck::pass("header", "header row matches expected format")
        } else {
            ValidationCheck::fail(
                "header",
                "header does not match expected BirdNET-Pi CSV format",
                false,
            )
        },
        if row_count > 0 {
            ValidationCheck::pass("row_count", format!("{row_count} data rows found"))
        } else {
            ValidationCheck::fail("row_count", "no data rows found", true)
        },
    ];

    let schema = DetectedSchema::BirdNetPiCsv { row_count };
    let report = ValidationReport::new("BirdNET-Pi CSV", row_count, checks);
    Ok((schema, report))
}

/// Return `true` if the path extension suggests CSV/TSV.
///
/// We also try to peek at the first bytes to distinguish `SQLite` (magic `SQLite format 3`)
/// from plain text.
fn is_csv_file(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_lowercase);

    if matches!(ext.as_deref(), Some("csv" | "tsv")) {
        return true;
    }

    // Peek: SQLite files begin with "SQLite format 3\0"
    if let Ok(mut f) = std::fs::File::open(path) {
        use std::io::Read;
        let mut magic = [0u8; 16];
        if f.read_exact(&mut magic).is_ok() {
            return &magic[..15] != b"SQLite format 3";
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::ProgressHandle;
    use rusqlite::Connection;
    use tempfile::NamedTempFile;

    fn make_sqlite_source(n: usize) -> NamedTempFile {
        let tmp = NamedTempFile::new().unwrap();
        let conn = Connection::open(tmp.path()).unwrap();
        conn.execute_batch(
            "CREATE TABLE detections (
            Date TEXT, Time TEXT, Sci_Name TEXT, Com_Name TEXT,
            Confidence REAL, Lat REAL, Lon REAL, Cutoff REAL,
            Week INTEGER, Sens REAL, Overlap REAL, File_Name TEXT);",
        )
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
    fn run_migration_sqlite_end_to_end() {
        let src = make_sqlite_source(20);
        let dst = NamedTempFile::new().unwrap();
        let progress = ProgressHandle::new();

        let summary = run_migration(src.path(), dst.path(), false, &progress).unwrap();
        assert_eq!(summary.source_rows, 20);
        assert_eq!(summary.imported_rows, 20);
    }

    #[test]
    fn validate_source_returns_schema_and_report() {
        let src = make_sqlite_source(5);
        let (schema, report, _) = validate_source(src.path()).unwrap();
        assert!(matches!(schema, DetectedSchema::BirdNetPi { .. }));
        assert!(report.passed);
    }

    #[test]
    fn csv_detected_by_extension() {
        use std::io::Write as _;
        let mut tmp = NamedTempFile::with_suffix(".csv").unwrap();
        writeln!(tmp, "Date\tTime\tSci_Name\tCom_Name\tConfidence").unwrap();
        writeln!(tmp, "2026-01-01\t06:00:00\tTurdus merula\tBlackbird\t0.9").unwrap();
        assert!(is_csv_file(tmp.path()));
    }
}
